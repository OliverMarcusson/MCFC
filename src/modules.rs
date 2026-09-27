//! Modules: every `.mcf` file is a module named by its path, like a Java
//! package (`src/game/util.mcf` is `game.util`), and `import` / `a.b.c(...)`
//! paths are resolved here.
//!
//! Files are merged into one source (so the rest of the pipeline is unchanged),
//! then `resolve` renames every item in a child module to its full path, e.g.
//! `double` in `src/util.mcf` becomes `util::double`, and rewrites calls,
//! records, and types to those names. Root-module items keep their bare
//! names, so single-file programs compile exactly as before.

use crate::lexer::TokenKind;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Span};

/// Where one module's file sits inside the merged source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSource {
    /// Path from the root module; empty for the root.
    pub path: Vec<String>,
    /// The module's file, or its directory when only subdirectories hold code.
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
    ("attribute.mcf", include_str!("../std/attribute.mcf")),
    ("color.mcf", include_str!("../std/color.mcf")),
    ("dialog.mcf", include_str!("../std/dialog.mcf")),
    ("list.mcf", include_str!("../std/list.mcf")),
    ("math.mcf", include_str!("../std/math.mcf")),
    ("noise.mcf", include_str!("../std/noise.mcf")),
    ("shape.mcf", include_str!("../std/shape.mcf")),
    ("str.mcf", include_str!("../std/str.mcf")),
    ("vec.mcf", include_str!("../std/vec.mcf")),
];

fn std_source(file: &Path) -> Option<&'static str> {
    let relative = file.strip_prefix(STD_ROOT).ok()?;
    STD_FILES
        .iter()
        .find(|(name, _)| Path::new(name) == relative)
        .map(|(_, source)| *source)
}

/// Loads `root` as the root module and each of `files` as the module its path
/// below `root`'s directory names, followed by `std`, into one merged source.
pub fn load(
    root: &Path,
    files: &[PathBuf],
    read: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<LoadedModules, String> {
    let dir = root.parent().unwrap_or(Path::new(""));
    // BTreeMap order puts every directory module before the modules inside it.
    let mut entries: BTreeMap<Vec<String>, PathBuf> = BTreeMap::new();
    for file in files.iter().filter(|file| file.as_path() != root) {
        let path = module_path(dir, file)?;
        if path[0] == "std" {
            return Err(format!(
                "error:{}: 'std' is reserved for the standard library",
                file.display()
            ));
        }
        entries.insert(path, file.clone());
    }
    for (name, _) in STD_FILES {
        let file = Path::new(STD_ROOT).join(name);
        entries.insert(
            module_path(Path::new(STD_ROOT), &file)?.into_iter().fold(
                vec!["std".to_string()],
                |mut path, segment| {
                    path.push(segment);
                    path
                },
            ),
            file,
        );
    }
    let paths: Vec<Vec<String>> = entries.keys().cloned().collect();
    for path in paths {
        for len in 1..path.len() {
            let parent = path[..len].to_vec();
            let folder = if parent[0] == "std" {
                Path::new(STD_ROOT).to_path_buf()
            } else {
                parent
                    .iter()
                    .fold(dir.to_path_buf(), |dir, segment| dir.join(segment))
            };
            entries.entry(parent).or_insert(folder);
        }
    }

    let mut loaded = LoadedModules {
        merged: String::new(),
        modules: Vec::new(),
    };
    load_module(root, Vec::new(), read, &mut loaded)?;
    for (path, file) in entries {
        load_module(&file, path, read, &mut loaded)?;
    }
    Ok(loaded)
}

/// `dir/game/util.mcf` -> `["game", "util"]`.
fn module_path(dir: &Path, file: &Path) -> Result<Vec<String>, String> {
    let relative = file.strip_prefix(dir).unwrap_or(file).with_extension("");
    let path: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let valid = |segment: &String| {
        segment.starts_with(|ch: char| ch == '_' || ch.is_ascii_alphabetic())
            && segment
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    };
    if path.is_empty() || !path.iter().all(valid) {
        return Err(format!(
            "error:{}: module file and folder names must be identifiers",
            file.display()
        ));
    }
    Ok(path)
}

fn load_module(
    file: &Path,
    path: Vec<String>,
    read: &dyn Fn(&Path) -> Result<String, String>,
    loaded: &mut LoadedModules,
) -> Result<(), String> {
    let is_file = std_source(file).is_some() || file.extension().is_some_and(|ext| ext == "mcf");
    let source = match std_source(file) {
        Some(source) => source.to_string(),
        None if is_file => read(file)?,
        None => String::new(),
    };
    loaded
        .merged
        .push_str(&format!("// source: {}\n", file.display()));
    loaded.modules.push(ModuleSource {
        path,
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
    Ok(())
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
    enum_names: HashSet<String>,
    /// Record name -> field names in declaration order, for `new R(...)`.
    record_fields: HashMap<String, Vec<String>>,
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

    // Every module is visible from everywhere, like a Java package.
    for index in 1..modules.len() {
        let name = modules[index].path.last().cloned().unwrap_or_default();
        if let Some(parent) = modules[index].parent {
            modules[parent].children.insert(name, (index, true));
        }
    }
    let function_modules: Vec<usize> = program
        .functions
        .iter()
        .map(|f| module_of(&f.span))
        .collect();
    let struct_modules: Vec<usize> = program.structs.iter().map(|s| module_of(&s.span)).collect();
    let enum_modules: Vec<usize> = program.enums.iter().map(|s| module_of(&s.span)).collect();
    for (function, &module) in program.functions.iter().zip(&function_modules) {
        modules[module]
            .functions
            .insert(function.name.clone(), function.is_pub);
    }
    for (def, &module) in program.structs.iter().zip(&struct_modules) {
        modules[module].structs.insert(def.name.clone(), def.is_pub);
    }
    for (def, &module) in program.enums.iter().zip(&enum_modules) {
        modules[module].structs.insert(def.name.clone(), def.is_pub);
    }

    let mut resolver = Resolver {
        modules,
        enum_names: HashSet::new(),
        record_fields: HashMap::new(),
    };
    resolver.enum_names = program
        .enums
        .iter()
        .zip(&enum_modules)
        .map(|(def, &module)| resolver.struct_name(module, &def.name))
        .collect();
    // Wildcards go last: like Java, names defined or imported by name win.
    let (wildcards, named): (Vec<_>, Vec<_>) =
        program.uses.iter().partition(|decl| decl.alias == "*");
    for decl in named.into_iter().chain(wildcards) {
        let module = module_of(&decl.span);
        if let Err(message) = resolver.add_import(module, decl) {
            diagnostics.push(Diagnostic::new(message, decl.span.clone()));
        }
    }

    for (def, &module) in program.structs.iter_mut().zip(&struct_modules) {
        for field in &mut def.fields {
            resolver.resolve_type(module, &[], &mut field.ty, &field.span, &mut diagnostics);
        }
        def.name = resolver.struct_name(module, &def.name);
        let fields = def.fields.iter().map(|field| field.name.clone()).collect();
        resolver.record_fields.insert(def.name.clone(), fields);
    }
    for (def, &module) in program.enums.iter_mut().zip(&enum_modules) {
        def.name = resolver.struct_name(module, &def.name);
    }
    for (function, &module) in program.functions.iter_mut().zip(&function_modules) {
        let generics = &function.type_params;
        for param in &mut function.params {
            resolver.resolve_type(
                module,
                generics,
                &mut param.ty,
                &param.span,
                &mut diagnostics,
            );
        }
        let span = function.span.clone();
        resolver.resolve_type(
            module,
            generics,
            &mut function.return_type,
            &span,
            &mut diagnostics,
        );
        let mut locals: HashSet<String> = function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        collect_locals(&function.body, &mut locals);
        let scope = Scope {
            module,
            generics,
            locals: &locals,
        };
        resolver.walk_stmts(&scope, &mut function.body, &mut diagnostics);
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
            format!("module '{}'", self.modules[module].path.join("."))
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
                    "{} '{}' is private to {}; mark it 'public'",
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

    /// Resolves `a.b.c` from `from`. The first segment is looked up in the
    /// current module, then the root module.
    fn resolve_path(&self, from: usize, segments: &[String]) -> Result<Vec<Target>, String> {
        let mut module = from;
        let first = &segments[0];
        let mut found = self.lookup(module, first, from)?;
        if found.is_empty() && from != 0 {
            found = self.lookup(0, first, from)?;
        }
        if found.is_empty() {
            return Err(format!(
                "cannot find '{}' in {}",
                first,
                self.display_path(module)
            ));
        }
        for (index, segment) in segments[1..].iter().enumerate() {
            let Some(next) = found.iter().find_map(|target| match target {
                Target::Module(module) => Some(*module),
                _ => None,
            }) else {
                return Err(format!(
                    "'{}' is not a module",
                    segments[..index + 1].join(".")
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
        }
        Ok(found)
    }

    fn add_import(&mut self, module: usize, decl: &UseDecl) -> Result<(), String> {
        let targets = self.resolve_path(module, &decl.path)?;
        if decl.alias == "*" {
            let Some(source) = targets.iter().find_map(|target| match target {
                Target::Module(source) => Some(*source),
                _ => None,
            }) else {
                return Err(format!("'{}' is not a module", decl.path.join(".")));
            };
            let entry = &self.modules[source];
            let names: BTreeSet<String> = entry
                .functions
                .iter()
                .chain(&entry.structs)
                .filter(|(_, is_pub)| **is_pub)
                .map(|(name, _)| name.clone())
                .collect();
            for name in names {
                let mut path = decl.path.clone();
                path.push(name.clone());
                // ponytail: a clash is skipped, not reported; two wildcards with the same name, first wins.
                let _ = self.add_import(
                    module,
                    &UseDecl {
                        path,
                        alias: name,
                        span: decl.span.clone(),
                    },
                );
            }
            return Ok(());
        }
        let entry = &mut self.modules[module];
        let alias = &decl.alias;
        // Modules, functions, and records are separate namespaces.
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
                    "cannot find function '{name}' in {}; it is defined in the root module, so add 'import {name};'",
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
            .ok_or_else(|| format!("'{}' is not a function", name.replace("::", ".")))
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
                    "cannot find record '{name}' in {}; it is defined in the root module, so add 'import {name};'",
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
            .ok_or_else(|| format!("'{}' is not a record", name.replace("::", ".")))
    }

    fn resolve_type(
        &self,
        module: usize,
        generics: &[String],
        ty: &mut Type,
        span: &Span,
        diagnostics: &mut Diagnostics,
    ) {
        match ty {
            Type::Struct(name) if generics.contains(name) => {}
            Type::Struct(name) => match self.resolve_struct(module, name) {
                Ok(Some(resolved)) => *name = resolved,
                Ok(None) => {}
                Err(message) => diagnostics.push(Diagnostic::new(message, span.clone())),
            },
            Type::Array(inner) | Type::Dict(inner) | Type::Optional(inner) => {
                self.resolve_type(module, generics, inner, span, diagnostics)
            }
            _ => {}
        }
    }

    fn walk_stmts(&self, scope: &Scope, stmts: &mut [Stmt], diagnostics: &mut Diagnostics) {
        for stmt in stmts {
            self.walk_stmt(scope, stmt, diagnostics);
        }
    }

    fn walk_stmt(&self, scope: &Scope, stmt: &mut Stmt, diagnostics: &mut Diagnostics) {
        match &mut stmt.kind {
            StmtKind::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    self.resolve_type(scope.module, scope.generics, ty, &stmt.span, diagnostics);
                }
                self.walk_expr(scope, value, diagnostics);
            }
            StmtKind::Assign { target, value } => {
                if let AssignTarget::Path(path) = target {
                    self.walk_path(scope, path, diagnostics);
                }
                self.walk_expr(scope, value, diagnostics);
            }
            StmtKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.walk_expr(scope, condition, diagnostics);
                self.walk_stmts(scope, then_body, diagnostics);
                self.walk_stmts(scope, else_body, diagnostics);
            }
            StmtKind::While {
                condition,
                body,
                step,
            } => {
                self.walk_expr(scope, condition, diagnostics);
                self.walk_stmts(scope, body, diagnostics);
                self.walk_stmts(scope, step, diagnostics);
            }
            StmtKind::For {
                ty, iterable, body, ..
            } => {
                if let Some(ty) = ty {
                    self.resolve_type(scope.module, scope.generics, ty, &stmt.span, diagnostics);
                }
                self.walk_expr(scope, iterable, diagnostics);
                self.walk_stmts(scope, body, diagnostics);
            }
            StmtKind::Switch {
                value,
                arms,
                default_body,
            } => {
                self.walk_expr(scope, value, diagnostics);
                for arm in arms {
                    self.walk_expr(scope, &mut arm.pattern, diagnostics);
                    self.walk_stmts(scope, &mut arm.body, diagnostics);
                }
                self.walk_stmts(scope, default_body, diagnostics);
            }
            StmtKind::Context { anchor, body, .. } => {
                self.walk_expr(scope, anchor, diagnostics);
                self.walk_stmts(scope, body, diagnostics);
            }
            StmtKind::Async { body } | StmtKind::Block(body) => {
                self.walk_stmts(scope, body, diagnostics)
            }
            StmtKind::Return(Some(value)) | StmtKind::Expr(value) => {
                self.walk_expr(scope, value, diagnostics)
            }
            StmtKind::MacroCommand(command) => self.resolve_placeholders(scope, command),
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::RawCommand(_) => {}
        }
    }

    /// The segments of `a.b` when `a` names a module rather than a local.
    fn module_path_of(&self, scope: &Scope, expr: &Expr) -> Option<Vec<String>> {
        let mut segments = Vec::new();
        let mut current = expr;
        loop {
            match &current.kind {
                ExprKind::Variable(name) => {
                    segments.push(name.clone());
                    break;
                }
                ExprKind::Path(path) => {
                    for segment in path.segments.iter().rev() {
                        let PathSegment::Field(name) = segment else {
                            return None;
                        };
                        segments.push(name.clone());
                    }
                    current = &path.base;
                }
                _ => return None,
            }
        }
        segments.reverse();
        let first = &segments[0];
        if scope.locals.contains(first) {
            return None;
        }
        let names_module = |module: usize| {
            self.lookup(module, first, scope.module)
                .ok()
                .is_some_and(|targets| targets.iter().any(|t| matches!(t, Target::Module(_))))
        };
        (names_module(scope.module) || names_module(0)).then_some(segments)
    }

    fn walk_path(&self, scope: &Scope, path: &mut PathExpr, diagnostics: &mut Diagnostics) {
        // `Mode.SURVIVAL`, or `game.Mode.SURVIVAL` through a module.
        let fields: Vec<String> = path
            .segments
            .iter()
            .map_while(|segment| match segment {
                PathSegment::Field(name) => Some(name.clone()),
                PathSegment::Index(_) => None,
            })
            .collect();
        if let ExprKind::Variable(base) = &path.base.kind
            && !scope.locals.contains(base)
        {
            for taken in 0..fields.len() {
                let name = std::iter::once(base.clone())
                    .chain(fields[..taken].iter().cloned())
                    .collect::<Vec<_>>()
                    .join("::");
                if let Ok(Some(resolved)) = self.resolve_struct(scope.module, &name)
                    && self.enum_names.contains(&resolved)
                {
                    path.base.kind = ExprKind::Variable(resolved);
                    path.segments.drain(..taken);
                    return;
                }
            }
        }
        self.walk_expr(scope, &mut path.base, diagnostics);
        for segment in &mut path.segments {
            if let PathSegment::Index(index) = segment {
                self.walk_expr(scope, index, diagnostics);
            }
        }
    }

    fn walk_expr(&self, scope: &Scope, expr: &mut Expr, diagnostics: &mut Diagnostics) {
        let span = expr.span.clone();
        // `util.double(x)` calls a function in module `util`.
        if let ExprKind::MethodCall {
            receiver,
            method,
            args,
        } = &mut expr.kind
            && let Some(mut segments) = self.module_path_of(scope, receiver)
        {
            // The parser maps Java method names to internal ones (`add` to `insert`)
            // before it knows the receiver is a module, so undo that for modules.
            let name = crate::language_catalog::java_method_names(method)
                .into_iter()
                .find(|java| {
                    let mut java_path = segments.clone();
                    java_path.push(java.to_string());
                    matches!(
                        self.resolve_function(scope.module, &java_path.join("::")),
                        Ok(Some(_))
                    )
                })
                .map_or_else(|| method.clone(), str::to_string);
            segments.push(name);
            let path = segments.join("::");
            let function = match self.resolve_function(scope.module, &path) {
                Ok(Some(resolved)) => resolved,
                Ok(None) => path,
                Err(message) => {
                    diagnostics.push(Diagnostic::new(message, span.clone()));
                    path
                }
            };
            expr.kind = ExprKind::Call {
                function,
                args: std::mem::take(args),
            };
            if let ExprKind::Call { args, .. } = &mut expr.kind {
                for arg in args {
                    self.walk_expr(scope, arg, diagnostics);
                }
            }
            return;
        }
        match &mut expr.kind {
            ExprKind::Call { function, args } => {
                match self.resolve_function(scope.module, function) {
                    Ok(Some(resolved)) => *function = resolved,
                    Ok(None) => {}
                    Err(message) => diagnostics.push(Diagnostic::new(message, span.clone())),
                }
                for arg in args {
                    self.walk_expr(scope, arg, diagnostics);
                }
            }
            ExprKind::StructLiteral { name, fields } => {
                self.resolve_record_name(scope, name, &span, diagnostics);
                for (_, value) in fields {
                    self.walk_expr(scope, value, diagnostics);
                }
            }
            ExprKind::New { name, args } => {
                self.resolve_record_name(scope, name, &span, diagnostics);
                for arg in args.iter_mut() {
                    self.walk_expr(scope, arg, diagnostics);
                }
                // `new R(a, b)` is the record literal with fields in declaration order.
                if let Some(fields) = self.record_fields.get(name) {
                    if fields.len() != args.len() {
                        diagnostics.push(Diagnostic::new(
                            format!(
                                "record '{}' has {} fields, found {} arguments",
                                name.replace("::", "."),
                                fields.len(),
                                args.len()
                            ),
                            span.clone(),
                        ));
                    }
                    expr.kind = ExprKind::StructLiteral {
                        name: std::mem::take(name),
                        fields: fields.iter().cloned().zip(std::mem::take(args)).collect(),
                    };
                }
            }
            ExprKind::ArrayLiteral(items) => {
                for item in items {
                    self.walk_expr(scope, item, diagnostics);
                }
            }
            ExprKind::DictLiteral(entries) => {
                for (_, value) in entries {
                    self.walk_expr(scope, value, diagnostics);
                }
            }
            ExprKind::Unary { expr, .. } => self.walk_expr(scope, expr, diagnostics),
            ExprKind::Binary { left, right, .. } => {
                self.walk_expr(scope, left, diagnostics);
                self.walk_expr(scope, right, diagnostics);
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.walk_expr(scope, receiver, diagnostics);
                for arg in args {
                    self.walk_expr(scope, arg, diagnostics);
                }
            }
            ExprKind::Path(path) => self.walk_path(scope, path, diagnostics),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.walk_expr(scope, condition, diagnostics);
                self.walk_expr(scope, then_expr, diagnostics);
                self.walk_expr(scope, else_expr, diagnostics);
            }
            ExprKind::Switch {
                value,
                arms,
                default,
            } => {
                self.walk_expr(scope, value, diagnostics);
                for (pattern, result) in arms {
                    self.walk_expr(scope, pattern, diagnostics);
                    self.walk_expr(scope, result, diagnostics);
                }
                if let Some(default) = default {
                    self.walk_expr(scope, default, diagnostics);
                }
            }
            ExprKind::String(text) => self.resolve_placeholders(scope, text),
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Variable(_) => {}
        }
    }

    /// `$(...)` placeholders are parsed later by the type checker, which
    /// doesn't know this module's imports, so each call in them is rewritten
    /// to its full name here: `$(twice(x))` becomes `$(util::twice(x))`.
    /// Unknown names are left for the type checker to report.
    fn resolve_placeholders(&self, scope: &Scope, text: &mut String) {
        if !text.contains("$(") {
            return;
        }
        let mut edits = Vec::new();
        for (start, end) in placeholder_ranges(text) {
            let Ok(tokens) = crate::lexer::lex(&text[start..end]) else {
                continue;
            };
            let name = |i: usize| match tokens.get(i).map(|token| &token.kind) {
                Some(TokenKind::Identifier(name)) => Some(name.clone()),
                _ => None,
            };
            for i in 0..tokens.len() {
                let after_dot =
                    i > 0 && matches!(tokens[i - 1].kind, TokenKind::Dot | TokenKind::Colon);
                let Some(first) = name(i).filter(|_| !after_dot) else {
                    continue;
                };
                if scope.locals.contains(&first) {
                    continue;
                }
                let mut segments = vec![first];
                let mut last = i;
                while matches!(tokens.get(last + 1).map(|t| &t.kind), Some(TokenKind::Dot))
                    && let Some(next) = name(last + 2)
                {
                    segments.push(next);
                    last += 2;
                }
                if !matches!(
                    tokens.get(last + 1).map(|t| &t.kind),
                    Some(TokenKind::LeftParen)
                ) {
                    continue;
                }
                if let Ok(Some(resolved)) =
                    self.resolve_function(scope.module, &segments.join("::"))
                {
                    edits.push((
                        start + tokens[i].range.start,
                        start + tokens[last].range.end,
                        resolved,
                    ));
                }
            }
        }
        for (from, to, resolved) in edits.into_iter().rev() {
            text.replace_range(from..to, &resolved);
        }
    }

    fn resolve_record_name(
        &self,
        scope: &Scope,
        name: &mut String,
        span: &Span,
        diagnostics: &mut Diagnostics,
    ) {
        match self.resolve_struct(scope.module, name) {
            Ok(Some(resolved)) => *name = resolved,
            Ok(None) => {}
            Err(message) => diagnostics.push(Diagnostic::new(message, span.clone())),
        }
    }
}

struct Scope<'a> {
    module: usize,
    generics: &'a [String],
    /// Every parameter and local name in the function; these shadow modules.
    locals: &'a HashSet<String>,
}

fn collect_locals(stmts: &[Stmt], locals: &mut HashSet<String>) {
    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Let { name, .. } => {
                locals.insert(name.clone());
            }
            StmtKind::For { name, body, .. } => {
                locals.insert(name.clone());
                collect_locals(body, locals);
            }
            StmtKind::If {
                then_body,
                else_body,
                ..
            } => {
                collect_locals(then_body, locals);
                collect_locals(else_body, locals);
            }
            StmtKind::While { body, step, .. } => {
                collect_locals(body, locals);
                collect_locals(step, locals);
            }
            StmtKind::Switch {
                arms, default_body, ..
            } => {
                for arm in arms {
                    collect_locals(&arm.body, locals);
                }
                collect_locals(default_body, locals);
            }
            StmtKind::Context { body, .. } | StmtKind::Async { body } | StmtKind::Block(body) => {
                collect_locals(body, locals)
            }
            _ => {}
        }
    }
}

/// Byte ranges of each `$(...)` body in `text`, skipping quoted parentheses.
fn placeholder_ranges(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if !(bytes[index] == b'$' && bytes[index + 1] == b'(') {
            index += 1;
            continue;
        }
        let start = index + 2;
        let (mut depth, mut quote) = (1, None);
        index = start;
        while index < bytes.len() {
            match (quote, bytes[index]) {
                (Some(_), b'\\') => index += 1,
                (Some(q), ch) if ch == q => quote = None,
                (Some(_), _) => {}
                (None, ch @ (b'"' | b'\'')) => quote = Some(ch),
                (None, b'(') => depth += 1,
                (None, b')') => {
                    depth -= 1;
                    if depth == 0 {
                        ranges.push((start, index));
                        break;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }
    ranges
}
