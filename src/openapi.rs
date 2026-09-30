//! The OpenAPI description served at `/api/openapi.json`.

use serde_json::{json, Value};

pub fn spec(public_url: &str) -> Value {
    let id = json!({ "name": "id", "in": "path", "required": true, "schema": { "type": "string" }, "description": "mod id, e.g. portal-gun" });
    let version = json!({ "name": "version", "in": "path", "required": true, "schema": { "type": "string" } });
    let pack = json!({ "name": "id", "in": "path", "required": true, "schema": { "type": "string" }, "description": "pack id" });
    let bearer = json!([{ "token": [] }]);
    let zip = json!({ "content": { "application/zip": { "schema": { "type": "string", "format": "binary" } } } });
    let ok = |d: &str| json!({ "description": d, "content": { "application/json": {} } });
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "OpenLina",
            "version": "1",
            "description": "Mods for Mosa Lina. Browse and vote without an account; upload with a token \
                            (`Authorization: Bearer olt_…`). Errors are `{\"error\": \"…\"}` with a 4xx/5xx status."
        },
        "servers": [{ "url": public_url }],
        "components": { "securitySchemes": { "token": { "type": "http", "scheme": "bearer" } } },
        "paths": {
            "/api/mods": {
                "get": {
                    "summary": "List mods (newest non-rejected version of each)",
                    "parameters": [
                        { "name": "section", "in": "query", "schema": { "enum": ["items", "modifiers", "levels", "general", "core"] } },
                        { "name": "q", "in": "query", "schema": { "type": "string" }, "description": "search id, name, description" },
                        { "name": "sort", "in": "query", "schema": { "enum": ["top", "new", "name"] } }
                    ],
                    "responses": { "200": ok("{mods: [mod], counts: {section: n}, game_build}") }
                },
                "post": {
                    "summary": "Upload a mod package (the zip `lina pack <id>` writes)",
                    "security": bearer,
                    "requestBody": zip,
                    "responses": {
                        "201": ok("{id, version, status, url, package}; non-admin uploads are `unreviewed`"),
                        "400": ok("invalid package"), "401": ok("no/unknown token"),
                        "403": ok("the mod id belongs to someone else"), "409": ok("version exists")
                    }
                }
            },
            "/api/mods/{id}": { "get": { "summary": "Mod details and version history", "parameters": [id], "responses": { "200": ok("mod + versions") } } },
            "/api/mods/{id}/vote": {
                "post": {
                    "summary": "Vote: one vote per mod per network (salted IP hash)",
                    "parameters": [id],
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "value": { "enum": [1, 0, -1] } } } } } },
                    "responses": { "200": ok("{id, score, my_vote}") }
                }
            },
            "/api/mods/{id}/{version}/package": { "get": { "summary": "Download a package", "parameters": [id, version], "responses": { "200": zip } } },
            "/api/mods/{id}/{version}/review": {
                "post": {
                    "summary": "Set the review status (admins)",
                    "security": bearer,
                    "parameters": [id, version],
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": { "status": { "enum": ["unreviewed", "reviewed", "rejected"] } } } } } },
                    "responses": { "200": ok("{id, version, status}") }
                }
            },
            "/api/packs": {
                "post": {
                    "summary": "Create a pack: pins versions, adds required mods, checks options; lists conflicting mods under `conflicts` (an agent resolves them)",
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object", "properties": {
                        "mods": { "type": "array", "items": { "type": "object", "required": ["id"], "properties": {
                            "id": { "type": "string" }, "version": { "type": "string" },
                            "options": { "type": "object" }, "request": { "type": "string", "description": "change request for an agent" } } } },
                        "section_requests": { "type": "object", "description": "{items|modifiers|levels|general: change request}" } } } } } },
                    "responses": { "201": ok("the pack (see GET /api/packs/{id})") }
                }
            },
            "/api/packs/{id}": { "get": { "summary": "A pack as JSON: mods with versions, package URLs, options, change requests", "parameters": [pack], "responses": { "200": ok("pack") } } },
            "/api/packs/{id}/zip": { "get": { "summary": "A pack as a zip for players (`openlina install`)", "parameters": [pack], "responses": { "200": zip } } },
            "/api/me": { "get": { "summary": "The token's user and their uploads", "security": bearer, "responses": { "200": ok("{name, admin, uploads}") } } },
            "/api/me/token": { "post": { "summary": "Replace the token (the old one stops working)", "security": bearer, "responses": { "200": ok("{name, token}") } } },
            "/api/review": { "get": { "summary": "Unreviewed uploads (admins)", "security": bearer, "responses": { "200": ok("{pending}") } } }
        }
    })
}
