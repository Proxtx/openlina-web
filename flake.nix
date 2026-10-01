{
  description = "openlina-web: the OpenLina mod hub";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # The kit release the site runs (its mod formats and kit version; the helper in pack zips
    # comes from the same release). Bump the tag, `helperHash` below and `nix flake update openlina-kit`
    # together.
    openlina-kit = {
      url = "github:Proxtx/openlina-kit/v0.1.0";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, rust-overlay, openlina-kit }:
    let
      kitVersion = "0.1.0";
      helperHash = "sha256-zPOUkwNJ34GP38lp50pt9K+QZLz4/4hISa6G2/vb7Hk=";
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Cargo.toml points at the sibling checkout (../openlina-kit), so build from a tree that has both.
      package = pkgs: pkgs.rustPlatform.buildRustPackage {
        pname = "openlina-web";
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
        src = pkgs.runCommand "openlina-src" { } ''
          mkdir -p $out/openlina-web $out/openlina-kit
          cp -r ${self}/. $out/openlina-web/
          cp -r ${openlina-kit}/. $out/openlina-kit/
          chmod -R u+w $out
        '';
        sourceRoot = "openlina-src/openlina-web";
        cargoLock.lockFile = ./Cargo.lock;
        doCheck = false; # `cargo test` in development; the server build stays quick on small hosts
        meta.mainProgram = "openlina-web";
      };

      # The players' helper from the kit release (Linux x86_64), served inside pack zips.
      helper = pkgs: pkgs.fetchurl {
        url = "https://github.com/Proxtx/openlina-kit/releases/download/v${kitVersion}/openlina-${kitVersion}-linux-x86_64";
        hash = helperHash;
      };
    in {
      packages = forAll (pkgs: {
        default = package pkgs;
        helper = helper pkgs;
      });

      # services.openlina-web: the site behind a reverse proxy on the same host.
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.openlina-web;
          args = lib.escapeShellArgs ([ "--data" cfg.dataDir "--public-url" cfg.publicUrl "--game-build" cfg.gameBuild ]);
          # `openlina-web-admin user-add <name> --admin`, `import <zip>…`, `review`: as the service user.
          admin = pkgs.writeShellScriptBin "openlina-web-admin" ''
            exec ${pkgs.util-linux}/bin/runuser -u openlina-web -- ${lib.getExe cfg.package} ${args} "$@"
          '';
        in {
          options.services.openlina-web = {
            enable = lib.mkEnableOption "the OpenLina mod hub";
            package = lib.mkOption {
              type = lib.types.package;
              default = package pkgs;
            };
            address = lib.mkOption { type = lib.types.str; default = "127.0.0.1"; };
            port = lib.mkOption { type = lib.types.port; default = 8080; };
            publicUrl = lib.mkOption {
              type = lib.types.str;
              example = "https://openlina.example.org";
              description = "Base URL in exported packs and API links.";
            };
            gameBuild = lib.mkOption { type = lib.types.str; default = "22056877"; };
            trustProxy = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Take voters' addresses from X-Forwarded-For (only behind your own proxy).";
            };
            dataDir = lib.mkOption { type = lib.types.str; default = "/var/lib/openlina-web"; };
            helpers = lib.mkOption {
              type = lib.types.attrsOf lib.types.path;
              default = { openlina = helper pkgs; };
              description = "Files put into the data dir's helpers/ (added to players' pack zips), by name.";
            };
          };

          config = lib.mkIf cfg.enable {
            users.users.openlina-web = { isSystemUser = true; group = "openlina-web"; home = cfg.dataDir; };
            users.groups.openlina-web = { };
            environment.systemPackages = [ admin ];

            systemd.services.openlina-web = {
              description = "OpenLina mod hub";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];
              preStart = ''
                mkdir -p ${cfg.dataDir}/helpers
                find ${cfg.dataDir}/helpers -maxdepth 1 -type l -delete
                ${lib.concatStrings (lib.mapAttrsToList (name: file: ''
                  ln -sfn ${file} ${cfg.dataDir}/helpers/${lib.escapeShellArg name}
                '') cfg.helpers)}
              '';
              serviceConfig = {
                User = "openlina-web";
                Group = "openlina-web";
                ExecStart = "${lib.getExe cfg.package} ${args} serve --addr ${cfg.address}:${toString cfg.port}"
                  + lib.optionalString cfg.trustProxy " --trust-proxy";
                Restart = "on-failure";
                StateDirectory = lib.mkIf (cfg.dataDir == "/var/lib/openlina-web") "openlina-web";
                ReadWritePaths = [ cfg.dataDir ];
                NoNewPrivileges = true;
                PrivateTmp = true;
                PrivateDevices = true;
                ProtectSystem = "strict";
                ProtectHome = true;
                ProtectKernelTunables = true;
                ProtectControlGroups = true;
                RestrictSUIDSGID = true;
              };
            };
          };
        };

      devShells = forAll (pkgs:
        let
          p = import nixpkgs { system = pkgs.stdenv.hostPlatform.system; overlays = [ rust-overlay.overlays.default ]; };
          rust = p.rust-bin.stable.latest.default.override { extensions = [ "rust-src" "clippy" "rustfmt" ]; };
        in {
          default = p.mkShell {
            packages = [ rust ];
            # Keep build outputs separate from a rustup toolchain's `target/`.
            CARGO_TARGET_DIR = "target/nix";
          };
        });
    };
}
