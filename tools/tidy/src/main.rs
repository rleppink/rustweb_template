//! # tidy — architecture checker for the rustweb workspace.
//!
//! Enforces the slice-boundary and wire-format rules from `AGENTS.md` /
//! `README.md` that the compiler and clippy cannot see:
//!
//! * **Import confinement** — inside `src/features/`, only `service.rs`
//!   files (and a small allowlist: `auth.rs`, `service_error.rs`,
//!   `stores/mapper.rs`, `test_support.rs`, `entities/`) touch SeaORM or
//!   entities; hand ers never may, which is what keeps handler-only slices
//!   honest (the moment a handler needs the DB it must grow a service);
//!   `HttpError` only lives in handlers, `error.rs`, and `auth.rs`.
//! * **Slice shape** — every `features/<feature>/<verb>/` directory has a
//!   `mod.rs` declaring `route()` (the wiring contract with the feature
//!   router); `handler.rs` and `service.rs` are conventions, not
//!   requirements — handler code is identified by its `#[utoipa::path]`
//!   attribute, service code by its SeaORM access. Feature modules declare
//!   `routes()`, verb modules declare `route()`.
//! * **Protected routes** — every `#[utoipa::path]` tagged `stores` must
//!   declare `security(("session_cookie" = []))`; no path may carry a user
//!   id (`{userId}`-style), sessions identify the caller.
//! * **Wire format** — `ToSchema` DTOs in `features/` serialize camelCase;
//!   entities never derive `Serialize` (so `password_hash` can't leak via a
//!   derived impl).
//! * **Env confinement** — `std::env` only in `config.rs` and
//!   `test_support.rs`; `config.rs` stays the single env-reading source.
//! * **OpenAPI drift** — `src/features/declared_paths.rs` (the const the
//!   router test iterates) is regenerated from the `#[utoipa::path]`
//!   attributes; `tidy` fails when it's stale.
//!
//! Usage: `cargo run -p tidy` (check), `cargo run -p tidy -- --update`
//! (regenerate `declared_paths.rs`). CI runs the check form.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::Span;
use syn::spanned::Spanned;

/// Files whose non-`handler.rs`/non-`service.rs` location may still touch
/// SeaORM or entities, measured relative to the workspace root.
///
/// `main.rs`/`state.rs` are infra wiring (the pool lives in `AppState`;
/// `main` converts the sqlx pool into a SeaORM connection), not slices —
/// the boundary rule targets slices, so they are exempt rather than forced
/// to disguise their plumbing.
const ENTITY_IMPORT_ALLOWLIST: &[&str] = &[
    "src/auth.rs",
    "src/service_error.rs",
    "src/main.rs",
    "src/state.rs",
    "src/features/stores/mapper.rs",
    "src/features/test_support.rs",
    "src/features/auth/mod.rs",
];

/// Files that may name `HttpError` besides `features/**/handler.rs`.
/// (`service_error.rs` only mentions it in a doc comment; the import check
/// sees real `use` statements, so it is not needed there.)
const HTTP_ERROR_ALLOWLIST: &[&str] = &["src/error.rs", "src/auth.rs"];

/// Files allowed to read the environment besides `src/config.rs`.
const ENV_ALLOWLIST: &[&str] = &["src/features/test_support.rs"];

/// Find the workspace root: the nearest ancestor of the current directory
/// that contains both a `Cargo.toml` and `src/features/` (tidy runs from the
/// workspace root or anywhere beneath it).
fn workspace_root() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join("Cargo.toml").is_file() && dir.join("src/features").is_dir() {
            return dir;
        }
        if !dir.pop() {
            return PathBuf::from(".");
        }
    }
}

/// Every `*.rs` file under `dir`, sorted for deterministic output.
fn rs_files_under(root: &Path, dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rs_files(&root.join(dir), &mut files);
    files.sort();
    files
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// A `use` import reduced to the paths it introduces, e.g.
/// `use crate::error::{ErrorResponses, HttpError};` yields the two leaf
/// paths `crate::error::ErrorResponses` and `crate::error::HttpError`.
fn import_leaves(use_item: &syn::ItemUse) -> Vec<(Vec<String>, String)> {
    fn walk(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<(Vec<String>, String)>) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                walk(&path.tree, prefix, out);
                prefix.pop();
            }
            syn::UseTree::Name(name) => out.push((prefix.clone(), name.ident.to_string())),
            syn::UseTree::Rename(rename) => out.push((prefix.clone(), rename.rename.to_string())),
            syn::UseTree::Glob(_) => out.push((prefix.clone(), "*".to_string())),
            syn::UseTree::Group(group) => {
                for child in &group.items {
                    walk(child, prefix, out);
                }
            }
        }
    }
    let mut leaves = Vec::new();
    walk(&use_item.tree, &mut Vec::new(), &mut leaves);
    leaves
}

fn check_imports(root: &Path, violations: &mut Vec<String>) {
    for file in rs_files_under(root, Path::new("src")) {
        let rel = file
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        let Some(parsed) = parse_file(&file, violations) else {
            continue;
        };
        let base_is_service = file.file_name().is_some_and(|name| name == "service.rs");
        let allow_entities = base_is_service
            || rel.starts_with("src/entities/")
            || ENTITY_IMPORT_ALLOWLIST.contains(&rel.as_str());
        // Handler code is contents, not a filename: `handler.rs` by
        // convention, or any file whose fn carries `#[utoipa::path]` (e.g.
        // a handler inlined next to `route()`). The HttpError confinement
        // follows the code either way.
        let allow_http = HTTP_ERROR_ALLOWLIST.contains(&rel.as_str())
            || file.file_name().is_some_and(|name| name == "handler.rs")
            || file_has_utoipa_path(&parsed);

        for item in &parsed.items {
            let syn::Item::Use(use_item) = item else {
                continue;
            };
            let line = use_item.span().start().line;
            for (prefix, leaf) in import_leaves(use_item) {
                let path = import_path_string(&prefix, &leaf);
                if (prefix.first().is_some_and(|s| s == "sea_orm")
                    || (prefix.first().is_some_and(|s| s == "crate")
                        && prefix.get(1).is_some_and(|s| s == "entities")))
                    && !allow_entities
                {
                    violations.push(format!(
                        "{rel}:{line}: SeaORM/entity import `{path}` outside a service \
                         (allowed in: services, {})",
                        ENTITY_IMPORT_ALLOWLIST.join(", ")
                    ));
                }
                if prefix == ["crate", "error"] && leaf == "HttpError" && !allow_http {
                    violations.push(format!(
                        "{rel}:{line}: `HttpError` import — handlers map `ServiceError` to \
                         `HttpError` at the slice boundary"
                    ));
                }
            }
        }
    }
}

fn import_path_string(prefix: &[String], leaf: &str) -> String {
    let mut path = prefix.join("::");
    if !path.is_empty() {
        path.push_str("::");
    }
    path.push_str(leaf);
    path
}

fn check_slice_shape(root: &Path, violations: &mut Vec<String>) {
    let features = root.join("src/features");
    let Ok(entries) = fs::read_dir(&features) else {
        violations.push(format!(
            "{}: missing src/features — run tidy from the workspace root",
            root.display()
        ));
        return;
    };
    for entry in entries.flatten() {
        let feature_dir = entry.path();
        if !feature_dir.is_dir() {
            continue;
        }
        let Some(feature_name) = feature_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let feature_mod = feature_dir.join("mod.rs");
        let Some(feature_parsed) = parse_file(&feature_mod, violations) else {
            continue;
        };
        if !declares_fn(&feature_parsed, "routes") {
            violations.push(format!(
                "features/{feature_name}/mod.rs: must declare `pub(crate) fn routes()` \
                 merging every verb's route()"
            ));
        }

        for verb_entry in fs::read_dir(&feature_dir).into_iter().flatten().flatten() {
            let verb_dir = verb_entry.path();
            if !verb_dir.is_dir() {
                continue; // feature-level files (mapper.rs, mod.rs) are fine
            }
            let Some(verb_name) = verb_dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // A slice's only file requirement is `mod.rs` — the module root
            // that declares `route()` and is `mod`-declared by the feature,
            // so a verb directory without it is dead code that never reaches
            // the router. `handler.rs` (HTTP surface) and `service.rs`
            // (domain logic) are conventions, not requirements: handler code
            // is any fn carrying `#[utoipa::path]`, wherever it lives, and
            // the import-confinement rule above is what forces `service.rs` —
            // the moment handler code needs SeaORM, it must move that access
            // into a `service.rs` (the only slice file allowed to). A
            // passthrough slice needs neither: a stub file to satisfy a
            // checker is worse than none.
            if !verb_dir.join("mod.rs").is_file() {
                violations.push(format!(
                    "features/{feature_name}/{verb_name}/: missing `mod.rs` \
                     (a slice's only required file: declares `route()` so the \
                     feature router can merge it; handler.rs/service.rs are \
                     optional conventions)"
                ));
            }
            if let Some(verb_parsed) = parse_file(&verb_dir.join("mod.rs"), violations)
                && !declares_fn(&verb_parsed, "route")
            {
                violations.push(format!(
                    "features/{feature_name}/{verb_name}/mod.rs: must declare \
                     `route()` returning the slice's OpenApiRouter"
                ));
            }
        }
    }
}

fn declares_fn(parsed: &syn::File, name: &str) -> bool {
    parsed.items.iter().any(|item| {
        matches!(
            item,
            syn::Item::Fn(function) if function.sig.ident == name
        )
    })
}

/// Parse a file; on failure record a violation and return `None` (an
/// unparseable source is itself a problem worth surfacing).
fn parse_file(path: &Path, violations: &mut Vec<String>) -> Option<syn::File> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            violations.push(format!("{}: unreadable source file: {err}", path.display()));
            return None;
        }
    };
    match syn::parse_file(&text) {
        Ok(parsed) => Some(parsed),
        Err(err) => {
            violations.push(format!("{}: failed to parse: {err}", path.display()));
            None
        }
    }
}

/// Extract `#[utoipa::path(...)]` facts from a parsed file: declared paths,
/// declared security, and tags.
struct UtopicFacts {
    paths: Vec<(Span, String)>,
    tags_without_security: Vec<Span>,
}

fn utoipa_facts(parsed: &syn::File) -> UtopicFacts {
    let mut facts = UtopicFacts {
        paths: Vec::new(),
        tags_without_security: Vec::new(),
    };
    for item in &parsed.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        for attr in &function.attrs {
            if !is_utoipa_path(attr.path()) {
                continue;
            }
            collect_utoipa_attr(attr, &mut facts);
        }
    }
    facts
}

fn is_utoipa_path(path: &syn::Path) -> bool {
    path.segments.len() == 2
        && path.segments.first().is_some_and(|s| s.ident == "utoipa")
        && path.segments.last().is_some_and(|s| s.ident == "path")
}

fn collect_utoipa_attr(attr: &syn::Attribute, facts: &mut UtopicFacts) {
    let mut tag: Option<String> = None;
    let mut declared_security = false;
    let mut declared_path: Option<(Span, String)> = None;

    // `parse_nested_meta` demands the whole attribute parse strictly, and
    // utoipa's `responses(...)` grammar is too loose for it. Iterate the
    // parenthesized token stream instead: `name`, `=`, `"value"` triples and
    // bare `security` idents. Values are read as raw `Literal`s (string
    // literals in attrs), so no parsing is needed.
    let Ok(list) = attr.meta.require_list() else {
        return;
    };
    let tokens: Vec<proc_macro2::TokenTree> = list.tokens.clone().into_iter().collect();
    let mut index = 0;
    while index < tokens.len() {
        let is_eq = |i: usize| {
            matches!(
                tokens.get(i),
                Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '='
            )
        };
        let is_str = |i: usize| {
            tokens
                .get(i)
                .is_some_and(|t| matches!(t, proc_macro2::TokenTree::Literal(_)))
        };
        if let Some(proc_macro2::TokenTree::Ident(ident)) = tokens.get(index) {
            match ident.to_string().as_str() {
                "path" | "tag" if is_eq(index + 1) && is_str(index + 2) => {
                    let value = match tokens.get(index + 2) {
                        Some(proc_macro2::TokenTree::Literal(lit)) => {
                            lit.to_string().trim_matches('"').to_string()
                        }
                        _ => String::new(),
                    };
                    if ident == "path" && value.starts_with('/') {
                        declared_path = Some((ident.span(), value));
                    } else if ident == "tag" {
                        tag = Some(value);
                    }
                    index += 3;
                    continue;
                }
                "security" => {
                    declared_security = true;
                    index += 1;
                    continue;
                }
                _ => {}
            }
        }
        index += 1;
    }

    if let Some((span, path)) = declared_path {
        facts.paths.push((span, path));
        if tag.as_deref() == Some("stores") && !declared_security {
            facts.tags_without_security.push(span);
        }
    }
}

/// Does this parsed file carry handler code — a fn with a `#[utoipa::path]`
/// attribute — anywhere? Handler code may live in `handler.rs` (the
/// convention) or beside `route()` in `mod.rs`; what makes it a handler is
/// the attribute, not the filename.
fn file_has_utoipa_path(parsed: &syn::File) -> bool {
    parsed.items.iter().any(|item| {
        let syn::Item::Fn(function) = item else {
            return false;
        };
        function
            .attrs
            .iter()
            .any(|attr| is_utoipa_path(attr.path()))
    })
}

fn check_utoipa(root: &Path, violations: &mut Vec<String>, update: bool) -> Result<(), String> {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut file_count = 0usize;

    // Handler code is identified by attribute, not filename, so every .rs
    // file under src/features is scanned (utopa path attrs never appear
    // anywhere else in the tree).
    for file in rs_files_under(root, Path::new("src/features")) {
        let rel = file
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        let Some(parsed) = parse_file(&file, violations) else {
            continue;
        };
        let facts = utoipa_facts(&parsed);
        if !facts.paths.is_empty() {
            file_count += 1;
        }
        for span in facts.tags_without_security.iter() {
            violations.push(format!(
                "{rel}:{}: stores slice must be protected: add \
                 `security((\"session_cookie\" = []))` to #[utoipa::path]",
                span.start().line
            ));
        }
        for (span, path) in &facts.paths {
            let normalized = normalize_path(path);
            declared.insert(normalized);
            if path.to_ascii_lowercase().contains("user") {
                violations.push(format!(
                    "{rel}:{}: user ids must not live in paths (the session \
                     identifies the caller): `{path}`",
                    span.start().line
                ));
            }
        }
    }

    let generated = render_declared_paths(&declared);
    let target = root.join("src/features/declared_paths.rs");
    if update {
        fs::write(&target, &generated)
            .map_err(|err| format!("failed to write {}: {err}", target.display()))?;
        println!("tidy: regenerated {}", target.display());
    } else {
        match fs::read_to_string(&target) {
            Ok(existing) if existing == generated => {}
            Ok(_) => violations.push(format!(
                "{}: stale — the declared #[utoipa::path] set has changed; \
                 run `cargo run -p tidy -- --update`",
                target.display()
            )),
            Err(_) => violations.push(format!(
                "{}: missing — run `cargo run -p tidy -- --update` to generate it",
                target.display()
            )),
        }
    }

    if file_count == 0 {
        violations.push("tidy: no #[utoipa::path] routes found under src/features".to_string());
    }
    Ok(())
}

/// Normalize a ref-relative utoipa path (e.g. `/stores/{id}`) to the served
/// form (e.g. `/api/stores/{id}`): everything the ref API serves sits under
/// `/api`, so the router test compares against the prefixed form.
fn normalize_path(path: &str) -> String {
    if path.starts_with("/api") {
        path.to_string()
    } else {
        format!("/api{path}")
    }
}

fn render_declared_paths(declared: &BTreeSet<String>) -> String {
    let mut out = String::from(
        "// @generated by tools/tidy — do not edit.\n\
         // Run `cargo run -p tidy -- --update` to regenerate.\n\
         //\n\
         // Every #[utoipa::path] route declared across the slices, normalized\n\
         // to the served form (/api prefix). The router test checks this set\n\
         // against the live spec, so doc and router cannot drift.\n\
         #[cfg(test)]\n\
         pub(crate) const DECLARED_API_PATHS: &[&str] = &[\n",
    );
    for path in declared {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("    \"{escaped}\",\n"));
    }
    out.push_str("];\n");
    out
}

fn check_dtos(root: &Path, violations: &mut Vec<String>) {
    for file in rs_files_under(root, Path::new("src/features")) {
        let rel = file
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        let Some(parsed) = parse_file(&file, violations) else {
            continue;
        };
        for item in &parsed.items {
            let syn::Item::Struct(item) = item else {
                continue;
            };
            if !derives(&item.attrs, "ToSchema") {
                continue;
            }
            let camel_case = struct_has_rename_all_camel(&item.attrs);
            for field in &item.fields {
                let Some(ident) = &field.ident else {
                    continue;
                };
                let name = ident.to_string();
                let single_word = !name.contains('_')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
                let renamed = field_has_serde_rename(&field.attrs);
                if !camel_case && !single_word && !renamed {
                    violations.push(format!(
                        "{rel}:{}: `{}` serializes snake_case `{name}` — DTOs are camelCase \
                         on the wire: add #[serde(rename_all = \"camelCase\")] or a \
                         #[serde(rename = ...)] on the field",
                        item.ident.span().start().line,
                        item.ident
                    ));
                }
            }
        }
    }
}

/// Does any `#[derive(...)]` attribute name `target` (last path segment)?
fn derives(attrs: &[syn::Attribute], target: &str) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }
        let Ok(list) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
        ) else {
            return false;
        };
        list.iter()
            .any(|p| p.segments.last().is_some_and(|s| s.ident == target))
    })
}

fn struct_has_rename_all_camel(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("serde") {
            return false;
        }
        let mut found = false;
        let outcome = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                found = lit.value() == "camelCase";
            }
            Ok(())
        });
        outcome.is_ok() && found
    })
}

fn field_has_serde_rename(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("serde") {
            return false;
        }
        let mut found = false;
        let outcome = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                found = !lit.value().is_empty();
            }
            Ok(())
        });
        outcome.is_ok() && found
    })
}

/// Entities must never derive `Serialize`, or a `password_hash`-bearing
/// entity could leak through a derived impl nobody wrote by hand.
fn check_entities_not_serializable(root: &Path, violations: &mut Vec<String>) {
    for file in rs_files_under(root, Path::new("src/entities")) {
        let rel = file
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        let Some(parsed) = parse_file(&file, violations) else {
            continue;
        };
        for item in &parsed.items {
            let syn::Item::Struct(item) = item else {
                continue;
            };
            if derives(&item.attrs, "Serialize") {
                violations.push(format!(
                    "{rel}:{}: entities must never derive `Serialize` — return DTOs, \
                     never entities (password_hash must not leak)",
                    item.ident.span().start().line
                ));
            }
        }
    }
}

fn check_env_confinement(root: &Path, violations: &mut Vec<String>) {
    for file in rs_files_under(root, Path::new("src")) {
        let rel = file
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "src/config.rs" || ENV_ALLOWLIST.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        if text.contains("std::env") {
            violations.push(format!(
                "{rel}: reads `std::env` — config.rs is the only env-reading source \
                 (typed Config, fail-fast)"
            ));
        }
    }
}

fn run(update: bool) -> Result<(), String> {
    let root = workspace_root();
    let mut violations: Vec<String> = Vec::new();

    check_slice_shape(&root, &mut violations);
    check_imports(&root, &mut violations);
    check_utoipa(&root, &mut violations, update)?;
    check_dtos(&root, &mut violations);
    check_entities_not_serializable(&root, &mut violations);
    check_env_confinement(&root, &mut violations);

    if violations.is_empty() {
        println!("tidy: all checks passed (root: {})", root.display());
        Ok(())
    } else {
        for violation in &violations {
            eprintln!("tidy: {violation}");
        }
        Err(format!(
            "tidy: {} violation(s) — see above",
            violations.len()
        ))
    }
}

fn main() {
    let update = std::env::args().any(|arg| arg == "--update");
    if let Err(message) = run(update) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
