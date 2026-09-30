//! Package files on disk and zip handling.
//!
//! ```text
//! <data>/openlina.db
//! <data>/packages/<id>/<version>.zip   uploaded packages, byte for byte
//! <data>/media/<id>/<version>/<file>   icon.png and gifs extracted from media/, served to browsers
//! <data>/helpers/                      optional `openlina` helper builds (e.g. openlina, openlina.exe),
//!                                      added to exported pack zips
//! ```
//!
//! A package zip is what `lina pack <id>` writes: `<id>/mod.toml`, `<id>/patch.wasm`, `<id>/assets/…`,
//! `<id>/media/…` (the files may also sit at the zip root).

use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::manifest::{ModManifest, ModPack};

pub const MAX_PACKAGE: usize = 32 << 20;
const MAX_UNPACKED: u64 = 96 << 20;
const MAX_ENTRIES: usize = 4096;
const MAX_MEDIA_FILE: u64 = 12 << 20;
const MAX_MEDIA_FILES: usize = 16;

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: &Path) -> Result<Self> {
        for d in ["packages", "media", "helpers"] {
            std::fs::create_dir_all(root.join(d)).with_context(|| format!("creating {}", root.join(d).display()))?;
        }
        Ok(Self { root: root.to_path_buf() })
    }

    pub fn package_path(&self, id: &str, version: &str) -> PathBuf {
        self.root.join("packages").join(id).join(format!("{version}.zip"))
    }

    pub fn media_path(&self, id: &str, version: &str, file: &str) -> PathBuf {
        self.root.join("media").join(id).join(version).join(file)
    }

    /// Save an inspected package and its media.
    pub fn save(&self, pkg: &Inspected, bytes: &[u8]) -> Result<()> {
        let (id, version) = (&pkg.manifest.info.id, &pkg.manifest.info.version);
        let path = self.package_path(id, version);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes)?;
        for (name, data) in &pkg.media {
            let p = self.media_path(id, version, name);
            std::fs::create_dir_all(p.parent().unwrap())?;
            std::fs::write(p, data)?;
        }
        Ok(())
    }

    /// Helper binaries to put into pack zips: (file name, path).
    pub fn helpers(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<_> = std::fs::read_dir(self.root.join("helpers"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
            .collect();
        out.sort();
        out
    }
}

/// A validated package.
pub struct Inspected {
    pub manifest: ModManifest,
    pub manifest_text: String,
    /// `media/` files the website shows: (file name, contents).
    pub media: Vec<(String, Vec<u8>)>,
    /// Folder the files sit in inside the zip (`"<id>/"` or `""`).
    pub prefix: String,
}

/// Check an uploaded package zip: safe paths, size limits, a valid `mod.toml` and a wasm patch.
pub fn inspect(bytes: &[u8]) -> Result<Inspected> {
    ensure!(bytes.len() <= MAX_PACKAGE, "package larger than {} MB", MAX_PACKAGE >> 20);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("not a zip file")?;
    ensure!(zip.len() <= MAX_ENTRIES, "too many files in the zip");
    let mut names = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let f = zip.by_index(i)?;
        let Some(p) = f.enclosed_name() else { bail!("unsafe path in zip: {}", f.name()) };
        ensure!(p.components().all(|c| matches!(c, Component::Normal(_))), "unsafe path in zip: {}", f.name());
        total += f.size();
        ensure!(total <= MAX_UNPACKED, "package unpacks to more than {} MB", MAX_UNPACKED >> 20);
        if f.is_file() {
            names.push(f.name().replace('\\', "/"));
        }
    }
    let prefix = if names.iter().any(|n| n == "mod.toml") {
        String::new()
    } else {
        let tops: Vec<&str> = names.iter().filter_map(|n| n.strip_suffix("/mod.toml")).filter(|t| !t.contains('/')).collect();
        match tops.as_slice() {
            [one] => format!("{one}/"),
            [] => bail!("no mod.toml in the package (expected <id>/mod.toml)"),
            _ => bail!("several mod.toml files; upload one mod per package"),
        }
    };
    ensure!(names.iter().all(|n| n.starts_with(&prefix)), "files outside `{prefix}` in the package");

    let manifest_text = read_file(&mut zip, &format!("{prefix}mod.toml"))?;
    let manifest_text = String::from_utf8(manifest_text).context("mod.toml is not UTF-8")?;
    let manifest = ModManifest::parse(&manifest_text).context("invalid mod.toml")?;
    let id = &manifest.info.id;
    ensure!(prefix.is_empty() || prefix == format!("{id}/"), "the package folder must be named after the mod id `{id}`");
    ensure!(valid_version(&manifest.info.version), "version `{}` is not like 1.2.3", manifest.info.version);
    ensure!(!manifest.info.name.trim().is_empty(), "the mod needs a name");
    let wasm = read_file(&mut zip, &format!("{prefix}patch.wasm")).context("the package needs patch.wasm (`lina pack` builds it)")?;
    ensure!(wasm.starts_with(b"\0asm"), "patch.wasm is not a WebAssembly module");

    let mut media = Vec::new();
    let media_dir = format!("{prefix}media/");
    for n in names.iter().filter(|n| n.starts_with(&media_dir)) {
        let file = &n[media_dir.len()..];
        let lower = file.to_ascii_lowercase();
        if file.contains('/') || !(lower.ends_with(".png") || lower.ends_with(".gif")) {
            continue;
        }
        ensure!(media.len() < MAX_MEDIA_FILES, "more than {MAX_MEDIA_FILES} media files");
        let f = zip.by_name(n)?;
        ensure!(f.size() <= MAX_MEDIA_FILE, "{file} is larger than {} MB", MAX_MEDIA_FILE >> 20);
        drop(f);
        media.push((file.to_string(), read_file(&mut zip, n)?));
    }
    media.sort();
    Ok(Inspected { manifest, manifest_text, media, prefix })
}

fn read_file(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut f = zip.by_name(name).with_context(|| format!("missing {name}"))?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// `1.2.3`, optionally with a `-pre` suffix.
pub fn valid_version(v: &str) -> bool {
    let (core, pre) = v.split_once('-').unwrap_or((v, "x"));
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
        && !pre.is_empty()
        && pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
        && v.len() <= 40
}

/// Order versions: numeric parts, a pre-release before its release.
pub fn version_key(v: &str) -> (Vec<u64>, bool, String) {
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre.is_none(), pre.unwrap_or("").to_string())
}

/// A pack zip for players: `openlina-pack/` with `modpack.toml`, `mods/<id>/`, the helpers and a
/// README. `mods` are (id, version) of stored packages. `openlina install <zip>` reads it.
pub fn pack_zip(store: &Store, pack: &ModPack, mods: &[(String, String)], readme: &str) -> Result<Vec<u8>> {
    let root = "openlina-pack";
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    out.start_file(format!("{root}/modpack.toml"), opts.unix_permissions(0o644))?;
    out.write_all(toml::to_string_pretty(pack)?.as_bytes())?;
    out.start_file(format!("{root}/README.txt"), opts.unix_permissions(0o644))?;
    out.write_all(readme.as_bytes())?;
    for (name, path) in store.helpers() {
        let mode = if name.ends_with(".exe") || !name.contains('.') { 0o755 } else { 0o644 };
        out.start_file(format!("{root}/{name}"), opts.unix_permissions(mode))?;
        out.write_all(&std::fs::read(&path)?)?;
    }
    for (id, version) in mods {
        let bytes = std::fs::read(store.package_path(id, version)).with_context(|| format!("package {id} {version}"))?;
        let prefix = inspect(&bytes)?.prefix;
        let mut zip = zip::ZipArchive::new(Cursor::new(&bytes[..]))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            if !f.is_file() {
                continue;
            }
            let name = f.name().replace('\\', "/");
            let rel = &name[prefix.len()..];
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            out.start_file(format!("{root}/mods/{id}/{rel}"), opts.unix_permissions(0o644))?;
            out.write_all(&buf)?;
        }
    }
    Ok(out.finish()?.into_inner())
}
