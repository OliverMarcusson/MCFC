//! Rust-style modules: loading the `mod` tree from disk and resolving `::` paths.
//!
//! Files are merged into one source (so the rest of the pipeline is unchanged),
//! then `resolve` renames every item in a child module to its full path, e.g.
//! `fn double` in `src/util.mcf` becomes `util::double`, and rewrites calls,
//! struct literals, and types to those names. Root-module items keep their bare
//! names, so single-file programs compile exactly as before.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};

/// Where one module's file sits inside the merged source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSource {
    /// Path from the root module; empty for the root.
    pub path: Vec<String>,
    pub file: PathBuf,
    /// Merged-source line (1-based) holding the file's first line.
    pub first_line: usize,
    /// Byte range of the file's text within the merged source.
    pub source_start: usize,
    pub source_end: usize,
}

#[derive(Debug, Clone)]
pub struct LoadedModules {
    pub merged: String,
    pub modules: Vec<ModuleSource>,
}

/// The standard library, compiled into the binary and loaded as module `std`.
const STD_ROOT: &str = "<std>";
const STD_FILES: &[(&str, &str)] = &[
    ("mod.mcf", include_str!("../std/mod.mcf")),
    ("math.mcf", include_str!("../std/math.mcf")),
];

fn std_source(file: &Path) -> Option<&'static str> {
    let relative = file.strip_prefix(STD_ROOT).ok()?;
    STD_FILES
        .iter()
        .find(|(name, _)| Path::new(name) == relative)
        .map(|(_, source)| *source)
}

fn exists(file: &Path) -> bool {
    std_source(file).is_some() || file.is_file()
}

/// Loads `root` plus every module reachable through `mod` declarations,
/// depth-first, into one merged source, followed by the `std` module.
pub fn load(
    root: &Path,
    read: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<LoadedModules, String> {
    let mut loaded = LoadedModules {
        merged: String::new(),
        modules: Vec::new(),
    };
    load_module(root, Vec::new(), read, &mut loaded)?;
    load_module(
        &Path::new(STD_ROOT).join("mod.mcf"),
        vec!["std".to_string()],
        read,
        &mut loaded,
    )?;
    Ok(loaded)
}

fn load_module(
    file: &Path,
    path: Vec<String>,
    read: &dyn Fn(&Path) -> Result<String, String>,
    loaded: &mut LoadedModules,
) -> Result<(), String> {
    let source = match std_source(file) {
        Some(source) => source.to_string(),
        None => read(file)?,
    };
    loaded
        .merged
        .push_str(&format!("# source: {}\n", file.display()));
    loaded.modules.push(ModuleSource {
        path: path.clone(),
        file: file.to_path_buf(),
        first_line: loaded.merged.matches('\n').count() + 1,
        source_start: loaded.merged.len(),
        source_end: 0,
    });
    let index = loaded.modules.len() - 1;
    loaded.merged.push_str(&source);
    if !source.ends_with('\n') {
        loaded.merged.push('\n');
    }
    loaded.modules[index].source_end = loaded.merged.len();
    loaded.merged.push('\n');

    // Like Rust: the root and `mod.mcf` own their directory; `foo.mcf` owns `foo/`.
    let dir = if path.is_empty() || file.file_stem().is_some_and(|stem| stem == "mod") {
        file.parent().unwrap_or(Path::new("")).to_path_buf()
    } else {
        file.with_extension("")
    };
    let mut seen = Vec::new();
    for (line, name) in declared_mods(&source) {
        if seen.contains(&name) {
            continue; // `resolve` reports the duplicate declaration.
        }
        seen.push(name.clone());
        if path.is_empty() && name == "std" {
            return Err(format!(
                "error:{}:{}: 'std' is reserved for the standard library",
                file.display(),
                line
            ));
        }
        let flat = dir.join(format!("{name}.mcf"));
        let nested = dir.join(&name).join("mod.mcf");
        let child = match (exists(&flat), exists(&nested)) {
            (true, false) => flat,
            (false, true) => nested,
            (true, true) => {
                return Err(format!(
                    "error:{}:{}: module '{}' is defined in both '{}' and '{}'",
                    file.display(),
                    line,
                    name,
                    flat.display(),
                    nested.display()
                ));
            }
            (false, false) => {
                return Err(format!(
                    "error:{}:{}: file not found for module '{}'; create '{}' or '{}'",
                    file.display(),
                    line,
                    name,
                    flat.display(),
                    nested.display()
                ));
            }
        };
        let mut child_path = path.clone();
        child_path.push(name);
        load_module(&child, child_path, read, loaded)?;
    }
    Ok(())
}

/// Top-level `mod name` / `pub mod name` lines, as (line number, name).
fn declared_mods(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.starts_with(char::is_whitespace))
        .filter_map(|(index, line)| {
            let line = line.split('#').next()?.trim();
            let line = line
                .strip_prefix("pub ")
                .map(str::trim_start)
                .unwrap_or(line);
            let name = line.strip_prefix("mod ")?.trim();
            let valid = name
                .chars()
                .next()
                .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
                && name
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
            valid.then(|| (index + 1, name.to_string()))
        })
        .collect()
}

#[derive(Debug, Clone)]
enum Target {
    Module(usize),
    Function(String),
    Struct(String),
}

#[derive(Debug, Default)]
struct Module {
    path: Vec<String>,
    parent: Option<usize>,
    /// name -> (module index, is_pub)
    children: HashMap<String, (usize, bool)>,
    /// local name -> is_pub
    functions: HashMap<String, bool>,
    structs: HashMap<String, bool>,
    imports: HashMap<String, Vec<Target>>,
}

struct Resolver {
    modules: Vec<Module>,
}

/// Renames child-module items to their full paths and resolves every path.
/// `sources` is empty for a single file without modules.
pub fn resolve(mut program: Program, sources: &[ModuleSource]) -> Result<Program, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let mut modules: Vec<Module> = if sources.is_empty() {
        vec![Module::default()]
    } else {
        sources
            .iter()
            .map(|source| Module {
                path: source.path.clone(),
                parent: sources.iter().position(|parent| {
                    !source.path.is_empty() && parent.path == source.path[..source.path.len() - 1]
                }),
                ..Module::default()
            })
            .collect()
    };
    let module_of = |span: &Span| {
        sources
            .iter()
            .rposition(|source| source.first_line <= span.line)
            .unwrap_or(0)
    };

    for decl in &program.mods {
        let owner = module_of(&decl.span);
        let mut path = modules[owner].path.clone();
        path.push(decl.name.clone());
        let Some(child) = modules.iter().position(|module| module.path == path) else {
            diagnostics.push(Diagnostic::new(
                format!(
                    "module '{}' was not loaded; 'mod' declarations need a file or project build",
                    decl.name
                ),
                decl.span.clone(),
            ));
            continue;
        };
        if modules[owner]
            .children
            .insert(decl.name.clone(), (child, decl.is_pub))
            .is_some()
        {
            diagnostics.push(Diagnostic::new(
                format!("module '{}' is declared twice", decl.name),
                decl.span.clone(),
            ));
        }
    }
    // `std` is implicitly a public child of the root, like Rust's prelude crate.
    if let Some(std) = modules.iter().position(|module| module.path == ["std"]) {
        modules[0]
            .children
            .entry("std".to_string())
            .or_insert((std, true));
    }
    let function_modules: Vec<usize> = program
        .functions
        .iter()
        .map(|f| module_of(&f.span))
        .collect();
    let struct_modules: Vec<usize> = program.structs.iter().map(|s| module_of(&s.span)).collect();
    for (function, &module) in program.functions.iter().zip(&function_modules) {
        modules[module]
            .functions
            .insert(function.name.clone(), function.is_pub);
    }
    for (def, &module) in program.structs.iter().zip(&struct_modules) {
        modules[module].structs.insert(def.name.clone(), def.is_pub);
    }

    let mut resolver = Resolver { modules };
    for decl in &program.uses {
        let module = module_of(&decl.span);
        if let Err(message) = resolver.add_import(module, decl) {
            diagnostics.push(Diagnostic::new(message, decl.span.clone()));
        }
    }

    for (def, &module) in program.structs.iter_mut().zip(&struct_modules) {
        for field in &mut def.fields {
            resolver.resolve_type(module, &mut field.ty, &field.span, &mut diagnostics);
        }
        def.name = resolver.struct_name(module, &def.name);
    }
    for (function, &module) in program.functions.iter_mut().zip(&function_modules) {
        for param in &mut function.params {
            resolver.resolve_type(module, &mut param.ty, &param.span, &mut diagnostics);
        }
        let span = function.span.clone();
        resolver.resolve_type(module, &mut function.return_type, &span, &mut diagnostics);
        resolver.walk_stmts(module, &mut function.body, &mut diagnostics);
        function.name = resolver.function_name(module, &function.name);
    }

    diagnostics.into_result(program)
}

impl Resolver {
    fn function_name(&self, module: usize, name: &str) -> String {
        // `tick` and desugared event/command/task handlers are hooks, not
        // callable items: they keep their bare names in every module.
        if module == 0 || name == "tick" || name.starts_with("__mcfc_") {
            name.to_string()
        } else {
            self.qualified(module, name)
        }
    }

    fn struct_name(&self, module: usize, name: &str) -> String {
        if module == 0 {
            name.to_string()
        } else {
            self.qualified(module, name)
        }
    }

    fn qualified(&self, module: usize, name: &str) -> String {
        let mut path = self.modules[module].path.clone();
        path.push(name.to_string());
        path.join("::")
    }

    fn display_path(&self, module: usize) -> String {
        if module == 0 {
            "the root module".to_string()
        } else {
            format!("module '{}'", self.modules[module].path.join("::"))
        }
    }

    /// Private items are visible inside their module and its descendants.
    fn visible(&self, from: usize, owner: usize, is_pub: bool) -> bool {
        let mut current = Some(from);
        while let Some(module) = current {
            if module == owner {
                return true;
            }
            current = self.modules[module].parent;
        }
        is_pub
    }

    /// Everything `name` refers to directly inside `module`, as seen from `from`.
    fn lookup(&self, module: usize, name: &str, from: usize) -> Result<Vec<Target>, String> {
        let entry = &self.modules[module];
        let mut found = Vec::new();
        let mut check = |is_pub: bool, kind: &str, target: Target| {
            if self.visible(from, module, is_pub) {
                found.push(target);
                Ok(())
            } else {
                Err(format!(
                    "{} '{}' is private to {}; mark it 'pub'",
                    kind,
                    name,
                    self.display_path(module)
                ))
            }
        };
        if let Some(&(child, is_pub)) = entry.children.get(name) {
            check(is_pub, "module", Target::Module(child))?;
        }
        if let Some(&is_pub) = entry.functions.get(name) {
            check(
                is_pub,
                "function",
                Target::Function(self.function_name(module, name)),
            )?;
        }
        if let Some(&is_pub) = entry.structs.get(name) {
            check(
                is_pub,
                "struct",
                Target::Struct(self.struct_name(module, name)),
            )?;
        }
        if module == from
            && let Some(targets) = entry.imports.get(name)
        {
            found.extend(targets.iter().cloned());
        }
        Ok(found)
    }

    /// Resolves `a::b::c` from `from`. The first segment is looked up in the
    /// current module, then the root module; `self` and `super` work as in Rust.
    fn resolve_path(&self, from: usize, segments: &[String]) -> Result<Vec<Target>, String> {
        let mut module = from;
        let mut index = 0;
        while index + 1 < segments.len() && matches!(segments[index].as_str(), "self" | "super") {
            if segments[index] == "super" {
                module = self.modules[module]
                    .parent
                    .ok_or("'super' cannot be used in the root module")?;
            }
            index += 1;
        }
        let first = &segments[index];
        let mut found = self.lookup(module, first, from)?;
        if found.is_empty() && index == 0 && from != 0 {
            found = self.lookup(0, first, from)?;
        }
        if found.is_empty() {
            return Err(format!(
                "cannot find '{}' in {}",
                first,
                self.display_path(module)
            ));
        }
        for segment in &segments[index + 1..] {
            let Some(next) = found.iter().find_map(|target| match target {
                Target::Module(module) => Some(*module),
                _ => None,
            }) else {
                return Err(format!(
                    "'{}' is not a module",
                    segments[..index + 1].join("::")
                ));
            };
            module = next;
            found = self.lookup(module, segment, from)?;
            if found.is_empty() {
                return Err(format!(
                    "cannot find '{}' in {}",
                    segment,
                    self.display_path(module)
                ));
            }
            index += 1;
        }
        Ok(found)
    }

    fn add_import(&mut self, module: usize, decl: &UseDecl) -> Result<(), String> {
        let targets = self.resolve_path(module, &decl.path)?;
        let entry = &mut self.modules[module];
        let alias = &decl.alias;
        // Modules, functions, and structs are separate namespaces, as in Rust.
        let imported = entry.imports.get(alias).map(Vec::as_slice).unwrap_or(&[]);
        let clash = targets.iter().any(|target| match target {
            Target::Module(_) => {
                entry.children.contains_key(alias)
                    || imported.iter().any(|t| matches!(t, Target::Module(_)))
            }
            Target::Function(_) => {
                entry.functions.contains_key(alias)
                    || imported.iter().any(|t| matches!(t, Target::Function(_)))
            }
            Target::Struct(_) => {
                entry.structs.contains_key(alias)
                    || imported.iter().any(|t| matches!(t, Target::Struct(_)))
            }
        });
        if clash {
            return Err(format!("'{alias}' is already defined in this module"));
        }
        entry
            .imports
            .entry(alias.clone())
            .or_default()
            .extend(targets);
        Ok(())
    }

    /// `None` leaves the name alone (builtins, or unknown names the type checker reports).
    fn resolve_function(&self, from: usize, name: &str) -> Result<Option<String>, String> {
        let segments: Vec<String> = name.split("::").map(str::to_string).collect();
        if segments.len() == 1 {
            let targets = self.lookup(from, name, from)?;
            if let Some(target) = targets.iter().find_map(|target| match target {
                Target::Function(name) => Some(name.clone()),
                _ => None,
            }) {
                return Ok(Some(target));
            }
            if from != 0 && self.modules[0].functions.contains_key(name) {
                return Err(format!(
                    "cannot find function '{name}' in {}; it is defined in the root module, so import it with 'use {name}'",
                    self.display_path(from)
                ));
            }
            return Ok(None);
        }
        self.resolve_path(from, &segments)?
            .into_iter()
            .find_map(|target| match target {
                Target::Function(name) => Some(Some(name)),
                _ => None,
            })
            .ok_or_else(|| format!("'{name}' is not a function"))
    }

    fn resolve_struct(&self, from: usize, name: &str) -> Result<Option<String>, String> {
        let segments: Vec<String> = name.split("::").map(str::to_string).collect();
        if segments.len() == 1 {
            let targets = self.lookup(from, name, from)?;
            if let Some(target) = targets.iter().find_map(|target| match target {
                Target::Struct(name) => Some(name.clone()),
                _ => None,
            }) {
                return Ok(Some(target));
            }
            if from != 0 && self.modules[0].structs.contains_key(name) {
                return Err(format!(
                    "cannot find struct '{name}' in {}; it is defined in the root module, so import it with 'use {name}'",
                    self.display_path(from)
                ));
            }
            return Ok(None);
        }
        self.resolve_path(from, &segments)?
            .into_iter()
            .find_map(|target| match target {
                Target::Struct(name) => Some(Some(name)),
                _ => None,
            })
            .ok_or_else(|| format!("'{name}' is not a struct"))
    }

    fn resolve_type(
        &self,
        module: usize,
        ty: &mut Type,
        span: &Span,
        diagnostics: &mut Diagnostics,
    ) {
        match ty {
            Type::Struct(name) => match self.resolve_struct(module, name) {
                Ok(Some(resolved)) => *name = resolved,
                Ok(None) => {}
                Err(message) => diagnostics.push(Diagnostic::new(message, span.clone())),
            },
            Type::Array(inner) | Type::Dict(inner) => {
                self.resolve_type(module, inner, span, diagnostics)
            }
            _ => {}
        }
    }

    fn walk_stmts(&self, module: usize, stmts: &mut [Stmt], diagnostics: &mut Diagnostics) {
        for stmt in stmts {
            self.walk_stmt(module, stmt, diagnostics);
        }
    }

    fn walk_stmt(&self, module: usize, stmt: &mut Stmt, diagnostics: &mut Diagnostics) {
        match &mut stmt.kind {
            StmtKind::Let { value, .. } => self.walk_expr(module, value, diagnostics),
            StmtKind::Assign { target, value } => {
                if let AssignTarget::Path(path) = target {
                    self.walk_path(module, path, diagnostics);
                }
                self.walk_expr(module, value, diagnostics);
            }
            StmtKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.walk_expr(module, condition, diagnostics);
                self.walk_stmts(module, then_body, diagnostics);
                self.walk_stmts(module, else_body, diagnostics);
            }
            StmtKind::While { condition, body } => {
                self.walk_expr(module, condition, diagnostics);
                self.walk_stmts(module, body, diagnostics);
            }
            StmtKind::For { kind, body, .. } => {
                match kind {
                    ForKind::Range { start, end, .. } => {
                        self.walk_expr(module, start, diagnostics);
                        self.walk_expr(module, end, diagnostics);
                    }
                    ForKind::Each { iterable } => self.walk_expr(module, iterable, diagnostics),
                }
                self.walk_stmts(module, body, diagnostics);
            }
            StmtKind::Match {
                value,
                arms,
                else_body,
            } => {
                self.walk_expr(module, value, diagnostics);
                for arm in arms {
                    self.walk_stmts(module, &mut arm.body, diagnostics);
                }
                self.walk_stmts(module, else_body, diagnostics);
            }
            StmtKind::Context { anchor, body, .. } => {
                self.walk_expr(module, anchor, diagnostics);
                self.walk_stmts(module, body, diagnostics);
            }
            StmtKind::Async { body } => self.walk_stmts(module, body, diagnostics),
            StmtKind::Return(Some(value)) | StmtKind::Expr(value) => {
                self.walk_expr(module, value, diagnostics)
            }
            // ponytail: `$(...)` placeholders are parsed later by the type checker,
            // so calls inside them must use the full path from the root module.
            // Resolve them here if relative paths in placeholders are needed.
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::RawCommand(_)
            | StmtKind::MacroCommand(_) => {}
        }
    }

    fn walk_path(&self, module: usize, path: &mut PathExpr, diagnostics: &mut Diagnostics) {
        self.walk_expr(module, &mut path.base, diagnostics);
        for segment in &mut path.segments {
            if let PathSegment::Index(index) = segment {
                self.walk_expr(module, index, diagnostics);
            }
        }
    }

    fn walk_expr(&self, module: usize, expr: &mut Expr, diagnostics: &mut Diagnostics) {
        let span = expr.span.clone();
        match &mut expr.kind {
            ExprKind::Call { function, args } => {
                match self.resolve_function(module, function) {
                    Ok(Some(resolved)) => *function = resolved,
                    Ok(None) => {}
                    Err(message) => diagnostics.push(Diagnostic::new(message, span.clone())),
                }
                for arg in args {
                    self.walk_expr(module, arg, diagnostics);
                }
            }
            ExprKind::StructLiteral { name, fields } => {
                match self.resolve_struct(module, name) {
                    Ok(Some(resolved)) => *name = resolved,
                    Ok(None) => {}
                    Err(message) => diagnostics.push(Diagnostic::new(message, span.clone())),
                }
                for (_, value) in fields {
                    self.walk_expr(module, value, diagnostics);
                }
            }
            ExprKind::Variable(name) if name.contains("::") => diagnostics.push(Diagnostic::new(
                format!("'{name}' is a path; only functions and structs can be named with '::'"),
                span.clone(),
            )),
            ExprKind::ArrayLiteral(items) => {
                for item in items {
                    self.walk_expr(module, item, diagnostics);
                }
            }
            ExprKind::DictLiteral(entries) => {
                for (_, value) in entries {
                    self.walk_expr(module, value, diagnostics);
                }
            }
            ExprKind::Unary { expr, .. } => self.walk_expr(module, expr, diagnostics),
            ExprKind::Binary { left, right, .. } => {
                self.walk_expr(module, left, diagnostics);
                self.walk_expr(module, right, diagnostics);
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.walk_expr(module, receiver, diagnostics);
                for arg in args {
                    self.walk_expr(module, arg, diagnostics);
                }
            }
            ExprKind::Path(path) => self.walk_path(module, path, diagnostics),
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::String(_) | ExprKind::Variable(_) => {}
        }
    }
}
