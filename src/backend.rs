use std::collections::BTreeMap;

use crate::ast::{BinaryOp, ContextKind, PathSegment, SleepUnit, Type, UnaryOp};
use crate::ir::{
    IrAssignTarget, IrCapture, IrExpr, IrExprKind, IrForKind, IrFunction, IrMacroPlaceholder,
    IrPathExpr, IrProgram, IrStmt,
};
use crate::types::{CastKind, RefKind};

#[derive(Debug, Clone)]
pub struct BuildArtifacts {
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct BackendOptions {
    pub namespace: String,
    pub load_tag_values: Option<Vec<String>>,
    pub tick_tag_values: Option<Vec<String>>,
    pub exports: Vec<ExportedFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedFunction {
    pub path: String,
    pub function: String,
}

pub fn generate(program: &IrProgram, options: &BackendOptions) -> BuildArtifacts {
    let mut backend = Backend::new(program, options.namespace.clone());
    backend.generate(program, options);
    BuildArtifacts {
        files: backend.files,
    }
}

struct Backend {
    namespace: String,
    files: BTreeMap<String, String>,
    functions: BTreeMap<String, FunctionInfo>,
    max_depth: usize,
    block_counter: usize,
    temp_counter: usize,
    macro_counter: usize,
    state_objectives: Vec<ManagedObjective>,
    block_builder_state_fields: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    params: Vec<(String, Type)>,
    return_type: Type,
    locals: BTreeMap<String, Type>,
}

#[derive(Debug, Clone)]
struct ManagedObjective {
    objective: String,
    display_name: Option<String>,
}

#[derive(Debug, Clone)]
struct Guard {
    ctrl_slot: String,
    break_slot: Option<String>,
    continue_slot: Option<String>,
}

#[derive(Debug, Clone)]
struct LoopContext {
    break_slot: String,
    continue_slot: String,
    continue_target: String,
}

#[derive(Debug, Clone)]
struct RenderedStoragePath {
    path: String,
    macro_storage: Option<String>,
}

#[derive(Debug, Clone)]
enum ContinuationItem {
    Stmt(IrStmt),
    Call {
        function_name: String,
        allow_continue: bool,
    },
    ClearScore(String),
    ExitContext,
}

#[derive(Debug, Clone)]
struct ContextResume {
    kind: ContextKind,
    anchor_slot: SlotRef,
}

impl Guard {
    fn for_function(depth: usize, function: &str) -> Self {
        Self {
            ctrl_slot: control_slot(depth, function),
            break_slot: None,
            continue_slot: None,
        }
    }

    fn within_loop(&self, loop_ctx: &LoopContext) -> Self {
        Self {
            ctrl_slot: self.ctrl_slot.clone(),
            break_slot: Some(loop_ctx.break_slot.clone()),
            continue_slot: Some(loop_ctx.continue_slot.clone()),
        }
    }

    fn wrap(&self, command: impl Into<String>) -> String {
        self.wrap_with_options(command, true, true)
    }

    fn wrap_allow_continue(&self, command: impl Into<String>) -> String {
        self.wrap_with_options(command, true, false)
    }

    fn wrap_with_options(
        &self,
        command: impl Into<String>,
        check_break: bool,
        check_continue: bool,
    ) -> String {
        let mut prefix = format!("execute if score {} mcfc matches 0", self.ctrl_slot);
        if check_break {
            if let Some(slot) = &self.break_slot {
                prefix.push_str(&format!(" if score {} mcfc matches 0", slot));
            }
        }
        if check_continue {
            if let Some(slot) = &self.continue_slot {
                prefix.push_str(&format!(" if score {} mcfc matches 0", slot));
            }
        }
        format!("{} run {}", prefix, command.into())
    }
}

impl Backend {
    fn new(program: &IrProgram, namespace: String) -> Self {
        let functions = program
            .functions
            .iter()
            .map(|function| {
                (
                    function.name.clone(),
                    FunctionInfo {
                        params: function
                            .params
                            .iter()
                            .map(|param| (param.name.clone(), param.ty.clone()))
                            .collect(),
                        return_type: function.return_type.clone(),
                        locals: function.locals.clone(),
                    },
                )
            })
            .collect();
        Self {
            namespace,
            files: BTreeMap::new(),
            functions,
            max_depth: program.call_depths.values().copied().max().unwrap_or(0) + 1,
            block_counter: 0,
            temp_counter: 0,
            macro_counter: 0,
            state_objectives: collect_state_objectives(program),
            block_builder_state_fields: collect_block_builder_state_fields(program),
        }
    }

    fn generate(&mut self, program: &IrProgram, options: &BackendOptions) {
        self.emit_pack_mcmeta();
        self.emit_load_tag(options.load_tag_values.as_deref());
        self.emit_tick_tag(program, options.tick_tag_values.as_deref());
        self.emit_setup();
        self.emit_main_entry();
        self.emit_tick_entry();
        for function in &program.functions {
            for depth in 0..=self.max_depth {
                self.emit_function_variant(function, depth);
            }
        }
        self.emit_auto_export_wrappers(program, &options.exports);
        self.emit_export_wrappers(&options.exports);
    }

    fn emit_pack_mcmeta(&mut self) {
        self.files.insert(
            "pack.mcmeta".to_string(),
            "{\n  \"pack\": {\n    \"min_format\": [101, 1],\n    \"max_format\": [101, 1],\n    \"description\": \"Generated by mcfc for Minecraft 26.1.2\"\n  }\n}\n"
                .to_string(),
        );
    }

    fn emit_load_tag(&mut self, override_values: Option<&[String]>) {
        let values = override_values
            .map(|items| items.to_vec())
            .unwrap_or_else(|| vec![format!("{}:main", self.namespace)]);
        self.files.insert(
            "data/minecraft/tags/function/load.json".to_string(),
            render_tag_file(&values),
        );
    }

    fn emit_tick_tag(&mut self, program: &IrProgram, override_values: Option<&[String]>) {
        let mut values = override_values
            .map(|items| items.to_vec())
            .unwrap_or_default();
        if has_special_tick(program) {
            let tick = format!("{}:tick", self.namespace);
            if !values.contains(&tick) {
                values.push(tick);
            }
        }
        if !values.is_empty() {
            self.files.insert(
                "data/minecraft/tags/function/tick.json".to_string(),
                render_tag_file(&values),
            );
        }
    }

    fn emit_setup(&mut self) {
        let mut lines = vec![
            "scoreboard objectives add mcfc dummy".to_string(),
            format!(
                "data modify storage {}:runtime frames set value {{}}",
                self.namespace
            ),
        ];
        for objective in &self.state_objectives {
            if let Some(display_name) = &objective.display_name {
                lines.push(format!(
                    "scoreboard objectives add {} dummy {}",
                    objective.objective,
                    quoted(display_name)
                ));
            } else {
                lines.push(format!(
                    "scoreboard objectives add {} dummy",
                    objective.objective
                ));
            }
        }

        for depth in 0..=self.max_depth {
            for (function, info) in &self.functions {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    control_slot(depth, function)
                ));
                for (name, ty) in &info.locals {
                    if matches!(ty, Type::Int | Type::Bool) {
                        lines.push(format!(
                            "scoreboard players set {} mcfc 0",
                            numeric_slot(depth, function, name)
                        ));
                    }
                }
                if matches!(info.return_type, Type::Int | Type::Bool) {
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        numeric_return_slot(depth, function)
                    ));
                }
            }
        }

        self.files.insert(
            format!(
                "data/{}/function/generated/setup.mcfunction",
                self.namespace
            ),
            lines.join("\n") + "\n",
        );
    }

    fn emit_main_entry(&mut self) {
        let mut lines = vec![format!("function {}:generated/setup", self.namespace)];
        if self.functions.contains_key("main") {
            lines.push(format!(
                "scoreboard players set {} mcfc 0",
                control_slot(0, "main")
            ));
            lines.push(format!(
                "function {}:{}",
                self.namespace,
                self.function_entry_name("main", 0)
            ));
        }
        self.files.insert(
            format!("data/{}/function/main.mcfunction", self.namespace),
            lines.join("\n") + "\n",
        );
    }

    fn emit_tick_entry(&mut self) {
        let Some(info) = self.functions.get("tick") else {
            return;
        };
        if !info.params.is_empty() || info.return_type != Type::Void {
            return;
        }
        let contents = format!(
            "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
            control_slot(0, "tick"),
            self.namespace,
            self.function_entry_name("tick", 0)
        );
        self.files.insert(
            format!("data/{}/function/tick.mcfunction", self.namespace),
            contents,
        );
    }

    fn emit_auto_export_wrappers(&mut self, program: &IrProgram, exports: &[ExportedFunction]) {
        for function in &program.functions {
            if function.generated
                || function.name == "main"
                || function.name == "tick"
                || !function.params.is_empty()
                || function.return_type != Type::Void
            {
                continue;
            }
            let relative = format!(
                "data/{}/function/{}.mcfunction",
                self.namespace, function.name
            );
            if exports
                .iter()
                .any(|export| export.path.trim_matches('/') == function.name)
            {
                continue;
            }
            if !self.files.contains_key(&relative) {
                let contents = format!(
                    "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
                    control_slot(0, &function.name),
                    self.namespace,
                    self.function_entry_name(&function.name, 0)
                );
                self.files.insert(relative, contents);
            }
        }
    }

    fn emit_export_wrappers(&mut self, exports: &[ExportedFunction]) {
        for export in exports {
            let path = export.path.trim_matches('/');
            if path.is_empty() {
                continue;
            }
            let relative = format!("data/{}/function/{}.mcfunction", self.namespace, path);
            let contents = format!(
                "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
                control_slot(0, &export.function),
                self.namespace,
                self.function_entry_name(&export.function, 0)
            );
            self.files.insert(relative, contents);
        }
    }

    fn emit_function_variant(&mut self, function: &IrFunction, depth: usize) {
        let path = self.function_entry_path(&function.name, depth);
        let mut lines = Vec::new();
        let guard = Guard::for_function(depth, &function.name);
        self.emit_stmt_list(
            function,
            depth,
            &function.body,
            &guard,
            None,
            None,
            &[],
            &mut lines,
        );
        self.files.insert(
            path,
            if lines.is_empty() {
                "# empty function\n".to_string()
            } else {
                lines.join("\n") + "\n"
            },
        );
    }

    fn emit_stmt_list(
        &mut self,
        function: &IrFunction,
        depth: usize,
        stmts: &[IrStmt],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        sleep_tail: &[ContinuationItem],
        lines: &mut Vec<String>,
    ) -> bool {
        for (index, stmt) in stmts.iter().enumerate() {
            let tail = continuation_after_stmts(&stmts[index + 1..], sleep_tail);
            match stmt {
                IrStmt::Let { name, value, .. } => {
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_named_slot(
                        function,
                        depth,
                        value,
                        name,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Assign { target, value } => {
                    let mut stmt_lines = Vec::new();
                    match target {
                        IrAssignTarget::Variable(name) => {
                            self.compile_expr_into_named_slot(
                                function,
                                depth,
                                value,
                                name,
                                &mut stmt_lines,
                            );
                        }
                        IrAssignTarget::Path(path) => {
                            self.compile_path_assign(function, depth, path, value, &mut stmt_lines);
                        }
                    }
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::RawCommand(raw) => lines.push(guard.wrap(expand_display_text_sugar(raw))),
                IrStmt::MacroCommand {
                    template,
                    placeholders,
                } => {
                    let mut stmt_lines = Vec::new();
                    self.emit_macro_command(
                        function,
                        depth,
                        template,
                        placeholders,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Sleep { duration, unit } => {
                    let continuation_name = self.emit_sleep_continuation(
                        function,
                        depth,
                        &tail,
                        guard,
                        loop_ctx,
                        resume_context,
                    );
                    let duration_name = self.new_temp();
                    let duration_slot =
                        local_slot(depth, &function.name, &duration_name, &Type::Int);
                    let macro_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_slot(
                        function,
                        depth,
                        duration,
                        &duration_slot,
                        &mut stmt_lines,
                    );
                    let duration_key = match unit {
                        SleepUnit::Seconds => "seconds",
                        SleepUnit::Ticks => "ticks",
                    };
                    stmt_lines.push(format!(
                        "execute store result storage {}:runtime {}.{} int 1 run scoreboard players get {} mcfc",
                        self.namespace,
                        macro_slot.storage_path(),
                        duration_key,
                        duration_slot.numeric_name()
                    ));
                    let placeholder = match unit {
                        SleepUnit::Seconds => "$(seconds)s",
                        SleepUnit::Ticks => "$(ticks)t",
                    };
                    stmt_lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        format!(
                            "schedule function {}:{} {}",
                            self.namespace, continuation_name, placeholder
                        ),
                    ));
                    stmt_lines.push(format!("scoreboard players set {} mcfc 1", guard.ctrl_slot));
                    self.extend_guarded(lines, guard, stmt_lines);
                    return true;
                }
                IrStmt::Context { kind, anchor, body } => {
                    let anchor_name = self.new_temp();
                    let anchor_slot = local_slot(depth, &function.name, &anchor_name, &anchor.ty);
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_slot(
                        function,
                        depth,
                        anchor,
                        &anchor_slot,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);

                    let mut context_tail = vec![ContinuationItem::ExitContext];
                    context_tail.extend(tail.clone());
                    let context_resume = ContextResume {
                        kind: *kind,
                        anchor_slot: anchor_slot.clone(),
                    };
                    let (body_path, body_name) = self.new_block(
                        function,
                        depth,
                        &format!("context_{}", context_execute_keyword(*kind)),
                    );
                    let mut body_lines = Vec::new();
                    self.emit_stmt_list(
                        function,
                        depth,
                        body,
                        guard,
                        loop_ctx,
                        Some(&context_resume),
                        &context_tail,
                        &mut body_lines,
                    );
                    self.files.insert(
                        body_path,
                        if body_lines.is_empty() {
                            "# empty context block\n".to_string()
                        } else {
                            body_lines.join("\n") + "\n"
                        },
                    );

                    lines.push(guard.wrap(self.query_command(
                        &anchor_slot,
                        format!(
                            "execute {} $(selector) run function {}:{}",
                            context_execute_keyword(*kind),
                            self.namespace,
                            body_name
                        ),
                        true,
                    )));
                }
                IrStmt::Async {
                    function: async_function,
                    captures,
                } => {
                    let mut stmt_lines = Vec::new();
                    self.emit_async_launch(
                        function,
                        depth,
                        async_function,
                        captures,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Expr(expr) => {
                    let scratch = self.new_temp();
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_named_slot(
                        function,
                        depth,
                        expr,
                        &scratch,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Return(expr) => {
                    let mut stmt_lines = Vec::new();
                    if let Some(expr) = expr {
                        let slot = return_slot(depth, &function.name, &expr.ty);
                        self.compile_expr_into_slot(function, depth, expr, &slot, &mut stmt_lines);
                    }
                    stmt_lines.push(format!(
                        "scoreboard players set {} mcfc 1",
                        control_slot(depth, &function.name)
                    ));
                    self.extend_guarded(lines, guard, stmt_lines);
                    return true;
                }
                IrStmt::Break => {
                    let Some(loop_ctx) = loop_ctx else {
                        continue;
                    };
                    lines.push(guard.wrap(format!(
                        "scoreboard players set {} mcfc 1",
                        loop_ctx.break_slot
                    )));
                    return true;
                }
                IrStmt::Continue => {
                    let Some(loop_ctx) = loop_ctx else {
                        continue;
                    };
                    lines.push(guard.wrap(format!(
                        "scoreboard players set {} mcfc 1",
                        loop_ctx.continue_slot
                    )));
                    return true;
                }
                IrStmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let cond_temp = self.new_temp();
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_named_slot(
                        function,
                        depth,
                        condition,
                        &cond_temp,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);

                    let (then_path, then_name) = self.new_block(function, depth, "if_then");
                    let mut then_lines = Vec::new();
                    self.emit_stmt_list(
                        function,
                        depth,
                        then_body,
                        guard,
                        loop_ctx,
                        resume_context,
                        &tail,
                        &mut then_lines,
                    );
                    self.files.insert(
                        then_path,
                        if then_lines.is_empty() {
                            "# empty if block\n".to_string()
                        } else {
                            then_lines.join("\n") + "\n"
                        },
                    );
                    lines.push(guard.wrap(format!(
                        "execute if score {} mcfc matches 1 run function {}:{}",
                        numeric_slot(depth, &function.name, &cond_temp),
                        self.namespace,
                        then_name
                    )));

                    if !else_body.is_empty() {
                        let (else_path, else_name) = self.new_block(function, depth, "if_else");
                        let mut else_lines = Vec::new();
                        self.emit_stmt_list(
                            function,
                            depth,
                            else_body,
                            guard,
                            loop_ctx,
                            resume_context,
                            &tail,
                            &mut else_lines,
                        );
                        self.files.insert(
                            else_path,
                            if else_lines.is_empty() {
                                "# empty else block\n".to_string()
                            } else {
                                else_lines.join("\n") + "\n"
                            },
                        );
                        lines.push(guard.wrap(format!(
                            "execute unless score {} mcfc matches 1 run function {}:{}",
                            numeric_slot(depth, &function.name, &cond_temp),
                            self.namespace,
                            else_name
                        )));
                    }
                }
                IrStmt::While { condition, body } => {
                    let break_slot = numeric_slot(depth, &function.name, &self.new_temp());
                    let continue_slot = numeric_slot(depth, &function.name, &self.new_temp());
                    let (cond_path, cond_name) = self.new_block(function, depth, "while_cond");
                    let (body_path, body_name) = self.new_block(function, depth, "while_body");

                    let loop_ctx = LoopContext {
                        break_slot: break_slot.clone(),
                        continue_slot: continue_slot.clone(),
                        continue_target: cond_name.clone(),
                    };
                    let loop_guard = guard.within_loop(&loop_ctx);
                    let mut body_sleep_tail = vec![
                        ContinuationItem::Call {
                            function_name: cond_name.clone(),
                            allow_continue: true,
                        },
                        ContinuationItem::ClearScore(break_slot.clone()),
                        ContinuationItem::ClearScore(continue_slot.clone()),
                    ];
                    body_sleep_tail.extend(tail.clone());

                    let mut cond_lines = vec![loop_guard.wrap_allow_continue(format!(
                        "scoreboard players set {} mcfc 0",
                        continue_slot
                    ))];
                    let cond_temp = self.new_temp();
                    let mut cond_eval = Vec::new();
                    self.compile_expr_into_named_slot(
                        function,
                        depth,
                        condition,
                        &cond_temp,
                        &mut cond_eval,
                    );
                    self.extend_guarded_allow_continue(&mut cond_lines, &loop_guard, cond_eval);
                    cond_lines.push(loop_guard.wrap_allow_continue(format!(
                        "execute if score {} mcfc matches 1 run function {}:{}",
                        numeric_slot(depth, &function.name, &cond_temp),
                        self.namespace,
                        body_name
                    )));

                    let mut body_lines = Vec::new();
                    self.emit_stmt_list(
                        function,
                        depth,
                        body,
                        &loop_guard,
                        Some(&loop_ctx),
                        resume_context,
                        &body_sleep_tail,
                        &mut body_lines,
                    );
                    body_lines.push(loop_guard.wrap_allow_continue(format!(
                        "function {}:{}",
                        self.namespace, loop_ctx.continue_target
                    )));

                    self.files.insert(cond_path, cond_lines.join("\n") + "\n");
                    self.files.insert(
                        body_path,
                        if body_lines.is_empty() {
                            "# empty while body\n".to_string()
                        } else {
                            body_lines.join("\n") + "\n"
                        },
                    );

                    lines.push(guard.wrap(format!("scoreboard players set {} mcfc 0", break_slot)));
                    lines.push(
                        guard.wrap(format!("scoreboard players set {} mcfc 0", continue_slot)),
                    );
                    lines.push(guard.wrap(format!("function {}:{}", self.namespace, cond_name)));
                }
                IrStmt::For { name, kind, body } => match kind {
                    IrForKind::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        let break_slot = numeric_slot(depth, &function.name, &self.new_temp());
                        let continue_slot = numeric_slot(depth, &function.name, &self.new_temp());
                        let end_name = self.new_temp();
                        let cond_temp = self.new_temp();
                        let (cond_path, cond_name) = self.new_block(function, depth, "for_cond");
                        let (body_path, body_name) = self.new_block(function, depth, "for_body");
                        let (step_path, step_name) = self.new_block(function, depth, "for_step");

                        let loop_ctx = LoopContext {
                            break_slot: break_slot.clone(),
                            continue_slot: continue_slot.clone(),
                            continue_target: step_name.clone(),
                        };
                        let loop_guard = guard.within_loop(&loop_ctx);
                        let mut body_sleep_tail = vec![
                            ContinuationItem::Call {
                                function_name: step_name.clone(),
                                allow_continue: true,
                            },
                            ContinuationItem::ClearScore(break_slot.clone()),
                            ContinuationItem::ClearScore(continue_slot.clone()),
                        ];
                        body_sleep_tail.extend(tail.clone());

                        let mut init_lines = Vec::new();
                        self.compile_expr_into_named_slot(
                            function,
                            depth,
                            start,
                            name,
                            &mut init_lines,
                        );
                        self.compile_expr_into_named_slot(
                            function,
                            depth,
                            end,
                            &end_name,
                            &mut init_lines,
                        );
                        self.extend_guarded(lines, guard, init_lines);
                        lines.push(
                            guard.wrap(format!("scoreboard players set {} mcfc 0", break_slot)),
                        );
                        lines.push(
                            guard.wrap(format!("scoreboard players set {} mcfc 0", continue_slot)),
                        );

                        let cmp_op = if *inclusive {
                            BinaryOp::Lte
                        } else {
                            BinaryOp::Lt
                        };
                        let cond_expr = IrExpr {
                            ty: Type::Bool,
                            ref_kind: RefKind::Unknown,
                            kind: IrExprKind::Binary {
                                op: cmp_op,
                                left: Box::new(IrExpr {
                                    ty: Type::Int,
                                    ref_kind: RefKind::Unknown,
                                    kind: IrExprKind::Variable(name.clone()),
                                }),
                                right: Box::new(IrExpr {
                                    ty: Type::Int,
                                    ref_kind: RefKind::Unknown,
                                    kind: IrExprKind::Variable(end_name.clone()),
                                }),
                            },
                        };

                        let mut cond_lines = Vec::new();
                        let mut cond_eval = Vec::new();
                        self.compile_expr_into_named_slot(
                            function,
                            depth,
                            &cond_expr,
                            &cond_temp,
                            &mut cond_eval,
                        );
                        self.extend_guarded(&mut cond_lines, &loop_guard, cond_eval);
                        cond_lines.push(loop_guard.wrap(format!(
                            "execute if score {} mcfc matches 1 run function {}:{}",
                            numeric_slot(depth, &function.name, &cond_temp),
                            self.namespace,
                            body_name
                        )));

                        let mut body_lines = Vec::new();
                        self.emit_stmt_list(
                            function,
                            depth,
                            body,
                            &loop_guard,
                            Some(&loop_ctx),
                            resume_context,
                            &body_sleep_tail,
                            &mut body_lines,
                        );
                        body_lines.push(loop_guard.wrap_allow_continue(format!(
                            "function {}:{}",
                            self.namespace, loop_ctx.continue_target
                        )));

                        let mut step_lines = vec![loop_guard.wrap_allow_continue(format!(
                            "scoreboard players set {} mcfc 0",
                            continue_slot
                        ))];
                        step_lines.push(loop_guard.wrap_allow_continue(format!(
                            "scoreboard players add {} mcfc 1",
                            numeric_slot(depth, &function.name, name)
                        )));
                        step_lines.push(loop_guard.wrap_allow_continue(format!(
                            "function {}:{}",
                            self.namespace, cond_name
                        )));

                        self.files.insert(cond_path, cond_lines.join("\n") + "\n");
                        self.files.insert(
                            body_path,
                            if body_lines.is_empty() {
                                "# empty for body\n".to_string()
                            } else {
                                body_lines.join("\n") + "\n"
                            },
                        );
                        self.files.insert(step_path, step_lines.join("\n") + "\n");

                        lines
                            .push(guard.wrap(format!("function {}:{}", self.namespace, cond_name)));
                    }
                    IrForKind::Each { iterable } => match &iterable.ty {
                        Type::EntitySet => {
                            let query_name = self.new_temp();
                            let mut init_lines = Vec::new();
                            self.compile_expr_into_named_slot(
                                function,
                                depth,
                                iterable,
                                &query_name,
                                &mut init_lines,
                            );
                            self.extend_guarded(lines, guard, init_lines);

                            let (body_path, body_name) =
                                self.new_block(function, depth, "for_each");
                            let mut body_lines = vec![
                                format!(
                                    "data modify storage {}:runtime {}.prefix set value \"\"",
                                    self.namespace,
                                    string_slot(depth, &function.name, name)
                                ),
                                format!(
                                    "data modify storage {}:runtime {}.selector set value \"@s\"",
                                    self.namespace,
                                    string_slot(depth, &function.name, name)
                                ),
                            ];
                            self.emit_stmt_list(
                                function,
                                depth,
                                body,
                                guard,
                                loop_ctx,
                                resume_context,
                                &tail,
                                &mut body_lines,
                            );
                            self.files.insert(body_path, body_lines.join("\n") + "\n");
                            lines.push(guard.wrap(self.query_command(
                                &local_slot(depth, &function.name, &query_name, &Type::EntitySet),
                                format!(
                                    "execute as $(selector) run function {}:{}",
                                    self.namespace, body_name
                                ),
                                true,
                            )));
                        }
                        Type::Array(element) => {
                            let snapshot_name = self.new_temp();
                            let index_name = self.new_temp();
                            let len_name = self.new_temp();
                            let break_slot = numeric_slot(depth, &function.name, &self.new_temp());
                            let continue_slot =
                                numeric_slot(depth, &function.name, &self.new_temp());
                            let (cond_path, cond_name) =
                                self.new_block(function, depth, "for_each_cond");
                            let (body_path, body_name) =
                                self.new_block(function, depth, "for_each_body");
                            let (step_path, step_name) =
                                self.new_block(function, depth, "for_each_step");
                            let loop_ctx = LoopContext {
                                break_slot: break_slot.clone(),
                                continue_slot: continue_slot.clone(),
                                continue_target: step_name.clone(),
                            };
                            let loop_guard = guard.within_loop(&loop_ctx);
                            let mut body_sleep_tail = vec![
                                ContinuationItem::Call {
                                    function_name: step_name.clone(),
                                    allow_continue: true,
                                },
                                ContinuationItem::ClearScore(break_slot.clone()),
                                ContinuationItem::ClearScore(continue_slot.clone()),
                            ];
                            body_sleep_tail.extend(tail.clone());

                            let mut init_lines = Vec::new();
                            self.compile_expr_into_named_slot(
                                function,
                                depth,
                                iterable,
                                &snapshot_name,
                                &mut init_lines,
                            );
                            init_lines.push(format!(
                                "scoreboard players set {} mcfc 0",
                                numeric_slot(depth, &function.name, &index_name)
                            ));
                            init_lines.push(format!(
                                "execute store result score {} mcfc run data get storage {}:runtime {}",
                                numeric_slot(depth, &function.name, &len_name),
                                self.namespace,
                                local_slot(
                                    depth,
                                    &function.name,
                                    &snapshot_name,
                                    &Type::Array(element.clone())
                                )
                                .storage_path()
                            ));
                            self.extend_guarded(lines, guard, init_lines);
                            lines.push(
                                guard.wrap(format!("scoreboard players set {} mcfc 0", break_slot)),
                            );
                            lines.push(
                                guard.wrap(format!(
                                    "scoreboard players set {} mcfc 0",
                                    continue_slot
                                )),
                            );

                            let cond_expr = IrExpr {
                                ty: Type::Bool,
                                ref_kind: RefKind::Unknown,
                                kind: IrExprKind::Binary {
                                    op: BinaryOp::Lt,
                                    left: Box::new(IrExpr {
                                        ty: Type::Int,
                                        ref_kind: RefKind::Unknown,
                                        kind: IrExprKind::Variable(index_name.clone()),
                                    }),
                                    right: Box::new(IrExpr {
                                        ty: Type::Int,
                                        ref_kind: RefKind::Unknown,
                                        kind: IrExprKind::Variable(len_name.clone()),
                                    }),
                                },
                            };
                            let cond_temp = self.new_temp();
                            let mut cond_lines = Vec::new();
                            let mut cond_eval = Vec::new();
                            self.compile_expr_into_named_slot(
                                function,
                                depth,
                                &cond_expr,
                                &cond_temp,
                                &mut cond_eval,
                            );
                            self.extend_guarded(&mut cond_lines, &loop_guard, cond_eval);
                            cond_lines.push(loop_guard.wrap(format!(
                                "execute if score {} mcfc matches 1 run function {}:{}",
                                numeric_slot(depth, &function.name, &cond_temp),
                                self.namespace,
                                body_name
                            )));

                            let mut body_lines = Vec::new();
                            let macro_storage = format!(
                                "frames.d{}.{}.__for_each{}",
                                depth,
                                sanitize(&function.name),
                                self.new_temp()
                            );
                            body_lines.push(format!(
                                "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                                self.namespace,
                                macro_storage,
                                numeric_slot(depth, &function.name, &index_name)
                            ));
                            let loop_slot =
                                local_slot(depth, &function.name, name, element.as_ref());
                            let command = match element.as_ref() {
                                Type::Int | Type::Bool => format!(
                                    "execute store result score {} mcfc run data get storage {}:runtime {}[$(index)] 1",
                                    loop_slot.numeric_name(),
                                    self.namespace,
                                    local_slot(
                                        depth,
                                        &function.name,
                                        &snapshot_name,
                                        &Type::Array(element.clone())
                                    )
                                    .storage_path()
                                ),
                                _ => format!(
                                    "data modify storage {}:runtime {} set from storage {}:runtime {}[$(index)]",
                                    self.namespace,
                                    loop_slot.storage_path(),
                                    self.namespace,
                                    local_slot(
                                        depth,
                                        &function.name,
                                        &snapshot_name,
                                        &Type::Array(element.clone())
                                    )
                                    .storage_path()
                                ),
                            };
                            body_lines
                                .push(self.storage_path_command(command, Some(macro_storage)));
                            self.emit_stmt_list(
                                function,
                                depth,
                                body,
                                &loop_guard,
                                Some(&loop_ctx),
                                resume_context,
                                &body_sleep_tail,
                                &mut body_lines,
                            );
                            body_lines.push(loop_guard.wrap_allow_continue(format!(
                                "function {}:{}",
                                self.namespace, loop_ctx.continue_target
                            )));

                            let mut step_lines = vec![loop_guard.wrap_allow_continue(format!(
                                "scoreboard players set {} mcfc 0",
                                continue_slot
                            ))];
                            step_lines.push(loop_guard.wrap_allow_continue(format!(
                                "scoreboard players add {} mcfc 1",
                                numeric_slot(depth, &function.name, &index_name)
                            )));
                            step_lines.push(loop_guard.wrap_allow_continue(format!(
                                "function {}:{}",
                                self.namespace, cond_name
                            )));

                            self.files.insert(cond_path, cond_lines.join("\n") + "\n");
                            self.files.insert(body_path, body_lines.join("\n") + "\n");
                            self.files.insert(step_path, step_lines.join("\n") + "\n");
                            lines.push(
                                guard.wrap(format!("function {}:{}", self.namespace, cond_name)),
                            );
                        }
                        _ => {}
                    },
                },
            }
        }
        false
    }

    fn emit_sleep_continuation(
        &mut self,
        function: &IrFunction,
        depth: usize,
        continuation: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
    ) -> String {
        let (path, name) = self.new_block(function, depth, "sleep_resume");
        let mut lines = vec![format!("scoreboard players set {} mcfc 0", guard.ctrl_slot)];
        self.emit_contextual_continuation_items(
            function,
            depth,
            continuation,
            guard,
            loop_ctx,
            resume_context,
            &mut lines,
        );
        self.files.insert(
            path,
            if lines.is_empty() {
                "# empty sleep continuation\n".to_string()
            } else {
                lines.join("\n") + "\n"
            },
        );
        name
    }

    fn emit_continuation_items(
        &mut self,
        function: &IrFunction,
        depth: usize,
        items: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        lines: &mut Vec<String>,
    ) -> bool {
        for (index, item) in items.iter().enumerate() {
            match item {
                ContinuationItem::Stmt(stmt) => {
                    if self.emit_stmt_list(
                        function,
                        depth,
                        std::slice::from_ref(stmt),
                        guard,
                        loop_ctx,
                        resume_context,
                        &items[index + 1..],
                        lines,
                    ) {
                        return true;
                    }
                }
                ContinuationItem::Call {
                    function_name,
                    allow_continue,
                } => {
                    let command = format!("function {}:{}", self.namespace, function_name);
                    lines.push(if *allow_continue {
                        guard.wrap_allow_continue(command)
                    } else {
                        guard.wrap(command)
                    });
                }
                ContinuationItem::ClearScore(slot) => {
                    lines.push(guard.wrap_with_options(
                        format!("scoreboard players set {} mcfc 0", slot),
                        false,
                        false,
                    ));
                }
                ContinuationItem::ExitContext => return false,
            }
        }
        false
    }

    fn emit_contextual_continuation_items(
        &mut self,
        function: &IrFunction,
        depth: usize,
        items: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(context) = resume_context else {
            return self
                .emit_continuation_items(function, depth, items, guard, loop_ctx, None, lines);
        };
        let split = items
            .iter()
            .position(|item| matches!(item, ContinuationItem::ExitContext))
            .unwrap_or(items.len());
        let inside = &items[..split];
        let outside = if split < items.len() {
            &items[split + 1..]
        } else {
            &[]
        };

        if !inside.is_empty() {
            let (path, name) = self.new_block(function, depth, "sleep_context");
            let mut inner_lines = Vec::new();
            self.emit_continuation_items(
                function,
                depth,
                inside,
                guard,
                loop_ctx,
                resume_context,
                &mut inner_lines,
            );
            self.files.insert(
                path,
                if inner_lines.is_empty() {
                    "# empty sleep context continuation\n".to_string()
                } else {
                    inner_lines.join("\n") + "\n"
                },
            );
            lines.push(guard.wrap(self.query_command(
                &context.anchor_slot,
                format!(
                    "execute {} $(selector) run function {}:{}",
                    context_execute_keyword(context.kind),
                    self.namespace,
                    name
                ),
                true,
            )));
        }

        self.emit_continuation_items(function, depth, outside, guard, loop_ctx, None, lines)
    }

    fn emit_async_launch(
        &mut self,
        parent: &IrFunction,
        parent_depth: usize,
        async_function: &IrFunction,
        captures: &[IrCapture],
        lines: &mut Vec<String>,
    ) {
        for capture in captures {
            let source = local_slot(parent_depth, &parent.name, &capture.name, &capture.ty);
            let target = local_slot(0, &async_function.name, &capture.name, &capture.ty);
            match capture.ty {
                Type::Int | Type::Bool => lines.push(format!(
                    "scoreboard players operation {} mcfc = {} mcfc",
                    target.numeric_name(),
                    source.numeric_name()
                )),
                Type::Void => {}
                _ => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    source.storage_path()
                )),
            }
        }
        lines.push(format!(
            "scoreboard players set {} mcfc 0",
            control_slot(0, &async_function.name)
        ));
        lines.push(format!(
            "function {}:{}",
            self.namespace,
            self.function_entry_name(&async_function.name, 0)
        ));
    }

    fn extend_guarded(&self, target: &mut Vec<String>, guard: &Guard, lines: Vec<String>) {
        target.extend(lines.into_iter().map(|line| guard.wrap(line)));
    }

    fn extend_guarded_allow_continue(
        &self,
        target: &mut Vec<String>,
        guard: &Guard,
        lines: Vec<String>,
    ) {
        target.extend(
            lines
                .into_iter()
                .map(|line| guard.wrap_allow_continue(line)),
        );
    }

    fn compile_expr_into_named_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        name: &str,
        lines: &mut Vec<String>,
    ) {
        let slot = local_slot(depth, &function.name, name, &expr.ty);
        self.compile_expr_into_slot(function, depth, expr, &slot, lines);
    }

    fn compile_expr_into_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        match &expr.kind {
            IrExprKind::Int(value) => lines.push(format!(
                "scoreboard players set {} mcfc {}",
                target.numeric_name(),
                value
            )),
            IrExprKind::Bool(value) => lines.push(format!(
                "scoreboard players set {} mcfc {}",
                target.numeric_name(),
                if *value { 1 } else { 0 }
            )),
            IrExprKind::String(value) => lines.push(format!(
                "data modify storage {}:runtime {} set value {}",
                self.namespace,
                target.storage_path(),
                quoted(value)
            )),
            IrExprKind::InterpolatedString {
                template,
                placeholders,
            } => self.compile_interpolated_string(
                function,
                depth,
                template,
                placeholders,
                target,
                lines,
            ),
            IrExprKind::ArrayLiteral(values) => {
                self.compile_array_literal(function, depth, values, target, lines);
            }
            IrExprKind::DictLiteral(entries) => {
                self.compile_dict_literal(function, depth, entries, target, lines);
            }
            IrExprKind::StructLiteral { fields, .. } => {
                self.compile_struct_literal(function, depth, fields, target, lines);
            }
            IrExprKind::Selector(value) => self.write_query_slot(target, "", value, lines),
            IrExprKind::Block(value) => self.write_block_slot(target, "", value, lines),
            IrExprKind::Variable(name) => match expr.ty {
                Type::Int | Type::Bool => lines.push(format!(
                    "scoreboard players operation {} mcfc = {} mcfc",
                    target.numeric_name(),
                    numeric_slot(depth, &function.name, name)
                )),
                Type::String
                | Type::Array(_)
                | Type::Dict(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::ItemDef
                | Type::TextDef
                | Type::ItemSlot
                | Type::Bossbar => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    string_slot(depth, &function.name, name)
                )),
                Type::EntitySet
                | Type::EntityRef
                | Type::PlayerRef
                | Type::BlockRef
                | Type::Nbt => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    string_slot(depth, &function.name, name)
                )),
                Type::Void => {}
            },
            IrExprKind::Single(expr) => {
                self.compile_expr_into_slot(function, depth, expr, target, lines);
            }
            IrExprKind::At { anchor, value } => {
                let anchor_name = self.new_temp();
                let value_name = self.new_temp();
                let anchor_slot = local_slot(depth, &function.name, &anchor_name, &anchor.ty);
                let value_slot = local_slot(depth, &function.name, &value_name, &value.ty);
                self.compile_expr_into_slot(function, depth, anchor, &anchor_slot, lines);
                self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
                self.compose_context_slots(
                    ContextKind::At,
                    &anchor_slot,
                    &value_slot,
                    target,
                    &value.ty,
                    lines,
                );
            }
            IrExprKind::As { anchor, value } => {
                let anchor_name = self.new_temp();
                let value_name = self.new_temp();
                let anchor_slot = local_slot(depth, &function.name, &anchor_name, &anchor.ty);
                let value_slot = local_slot(depth, &function.name, &value_name, &value.ty);
                self.compile_expr_into_slot(function, depth, anchor, &anchor_slot, lines);
                self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
                self.compose_context_slots(
                    ContextKind::As,
                    &anchor_slot,
                    &value_slot,
                    target,
                    &value.ty,
                    lines,
                );
            }
            IrExprKind::Exists(expr) => {
                let temp = self.new_temp();
                let source_slot = local_slot(depth, &function.name, &temp, &expr.ty);
                self.compile_expr_into_slot(function, depth, expr, &source_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(self.query_command(
                    &source_slot,
                    format!(
                        "execute if entity $(selector) run scoreboard players set {} mcfc 1",
                        target.numeric_name()
                    ),
                    true,
                ));
            }
            IrExprKind::HasData(expr) => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, expr, lines)
                {
                    lines.push(self.storage_path_command(
                        format!(
                            "execute if data storage {}:runtime {} run scoreboard players set {} mcfc 1",
                            self.namespace,
                            rendered.path,
                            target.numeric_name()
                        ),
                        rendered.macro_storage,
                    ));
                }
            }
            IrExprKind::Path(path) => {
                self.compile_path_read(function, depth, path, target, lines);
            }
            IrExprKind::Cast { kind, expr } => {
                self.compile_cast(function, depth, *kind, expr, target, lines);
            }
            IrExprKind::Unary { op, expr } => {
                self.compile_unary(function, depth, *op, expr, target, lines);
            }
            IrExprKind::Binary { op, left, right } => {
                self.compile_binary(function, depth, *op, left, right, target, lines);
            }
            IrExprKind::Call {
                function: callee,
                args,
            } => {
                if self.compile_builtin_call(function, depth, callee, args, target, lines) {
                    return;
                }
                let callee_depth = depth + 1;
                if let Some(info) = self.functions.get(callee).cloned() {
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        control_slot(callee_depth, callee)
                    ));
                    for ((param_name, param_ty), arg) in info.params.iter().zip(args.iter()) {
                        let param_slot = local_slot(callee_depth, callee, param_name, param_ty);
                        self.compile_expr_into_slot(function, depth, arg, &param_slot, lines);
                    }
                    lines.push(format!(
                        "function {}:{}",
                        self.namespace,
                        self.function_entry_name(callee, callee_depth)
                    ));
                    match expr.ty {
                        Type::Int | Type::Bool => lines.push(format!(
                            "scoreboard players operation {} mcfc = {} mcfc",
                            target.numeric_name(),
                            numeric_return_slot(callee_depth, callee)
                        )),
                        Type::String
                        | Type::Array(_)
                        | Type::Dict(_)
                        | Type::Struct(_)
                        | Type::EntityDef
                        | Type::BlockDef
                        | Type::ItemDef
                        | Type::TextDef
                        | Type::ItemSlot
                        | Type::Bossbar => lines.push(format!(
                            "data modify storage {}:runtime {} set from storage {}:runtime {}",
                            self.namespace,
                            target.storage_path(),
                            self.namespace,
                            string_return_slot(callee_depth, callee)
                        )),
                        Type::EntitySet
                        | Type::EntityRef
                        | Type::PlayerRef
                        | Type::BlockRef
                        | Type::Nbt => lines.push(format!(
                            "data modify storage {}:runtime {} set from storage {}:runtime {}",
                            self.namespace,
                            target.storage_path(),
                            self.namespace,
                            string_return_slot(callee_depth, callee)
                        )),
                        Type::Void => {}
                    }
                }
            }
            IrExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                self.compile_method_call(function, depth, receiver, method, args, target, lines);
            }
        }
    }

    fn compile_path_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let base_name = self.new_temp();
        let base_slot = local_slot(depth, &function.name, &base_name, &path.base.ty);
        self.compile_expr_into_slot(function, depth, &path.base, &base_slot, lines);

        let value_name = self.new_temp();
        let value_slot = local_slot(depth, &function.name, &value_name, &Type::Nbt);
        self.compile_value_as_nbt(function, depth, value, &value_slot, lines);

        if path.base.ty == Type::Bossbar {
            self.compile_bossbar_property_assign(function, depth, &base_slot, path, value, lines);
            return;
        }

        if path.base.ty == Type::ItemSlot {
            self.compile_item_slot_path_assign(
                function,
                depth,
                &base_slot,
                path,
                &value_slot,
                lines,
            );
            return;
        }

        if matches!(
            path.base.ty,
            Type::Array(_)
                | Type::Dict(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::ItemDef
                | Type::TextDef
                | Type::BlockDef
                | Type::Nbt
        ) {
            if self.try_compile_storage_index_assign(function, depth, path, &value_slot, lines) {
                return;
            }
            if let Some(rendered) = self.render_storage_lvalue_path(function, depth, path, lines) {
                lines.push(self.storage_path_command(
                    format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        rendered.path,
                        self.namespace,
                        value_slot.storage_path()
                    ),
                    rendered.macro_storage,
                ));
            }
            return;
        }

        if matches!(path.base.ty, Type::EntityRef | Type::PlayerRef) {
            if let Some(PathSegment::Field(first)) = path.segments.first() {
                if first == "position" && path.segments.len() > 1 {
                    let pos_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::BlockRef);
                    self.compose_entity_position_slot(&base_slot, &pos_slot, lines);
                    let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
                        &Type::BlockRef,
                        &path.segments[1..],
                    ));
                    let storage_target =
                        format!("{}:runtime {}", self.namespace, value_slot.storage_path());
                    lines.push(self.block_command(
                        &pos_slot,
                        format!(
                            "data modify block $(pos) {} set from storage {}",
                            path_text, storage_target
                        ),
                        true,
                    ));
                    return;
                }
            }
            if self.try_compile_player_path_assign(
                function,
                depth,
                &base_slot,
                path,
                value,
                &value_slot,
                lines,
            ) {
                return;
            }
        }

        let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
            &path.base.ty,
            &path.segments,
        ));
        let storage_target = format!("{}:runtime {}", self.namespace, value_slot.storage_path());
        match path.base.ty {
            Type::EntityRef | Type::PlayerRef => lines.push(self.query_command(
                &base_slot,
                format!(
                    "data modify entity $(selector) {} set from storage {}",
                    path_text, storage_target
                ),
                true,
            )),
            Type::BlockRef => lines.push(self.block_command(
                &base_slot,
                format!(
                    "data modify block $(pos) {} set from storage {}",
                    path_text, storage_target
                ),
                true,
            )),
            _ => {}
        }
    }

    fn compile_bossbar_property_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let [PathSegment::Field(field)] = path.segments.as_slice() else {
            return;
        };
        match field.as_str() {
            "name" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                if let IrExprKind::String(text) = &value.kind {
                    let component = selector_text_components(text).unwrap_or_else(|| quoted(text));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        format!("bossbar set $(id) name {}", component),
                    ));
                    return;
                }
                if value.ty == Type::TextDef {
                    let name_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
                    self.compile_expr_into_slot(function, depth, value, &name_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                        self.namespace,
                        macro_slot.storage_path(),
                        self.namespace,
                        name_slot.storage_path()
                    ));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        "bossbar set $(id) name $(name)".to_string(),
                    ));
                    return;
                }
                let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, value, &name_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    name_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "bossbar set $(id) name [\"$(name)\"]".to_string(),
                ));
            }
            "value" | "max" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let value_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.value int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    value_slot.numeric_name()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!("bossbar set $(id) {} $(value)", field),
                ));
            }
            "visible" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let visible_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
                self.compile_expr_into_slot(function, depth, value, &visible_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.visible int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    visible_slot.numeric_name()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "bossbar set $(id) visible $(visible)".to_string(),
                ));
            }
            "players" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &value.ty);
                self.compile_expr_into_slot(function, depth, value, &target_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "bossbar set $(id) players $(selector)".to_string(),
                    true,
                ));
            }
            _ => {}
        }
    }

    fn compile_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let base_name = self.new_temp();
        let base_slot = local_slot(depth, &function.name, &base_name, &path.base.ty);
        self.compile_expr_into_slot(function, depth, &path.base, &base_slot, lines);

        if path.base.ty == Type::ItemSlot {
            self.compile_item_slot_path_read(function, depth, &base_slot, path, target, lines);
            return;
        }

        if matches!(
            path.base.ty,
            Type::Array(_)
                | Type::Dict(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::Nbt
        ) {
            let path_text = self.render_storage_read_path(function, depth, path, &base_slot, lines);
            self.compile_storage_read_from_path(path_text, &path.ty, target, lines);
            return;
        }

        if matches!(path.base.ty, Type::EntityRef | Type::PlayerRef) {
            if self.try_compile_player_path_read(function, depth, &base_slot, path, target, lines) {
                return;
            }
        }

        let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
            &path.base.ty,
            &path.segments,
        ));
        match path.base.ty {
            Type::EntityRef | Type::PlayerRef => lines.push(self.query_command(
                &base_slot,
                format!(
                    "data modify storage {}:runtime {} set from entity $(selector) {}",
                    self.namespace,
                    target.storage_path(),
                    path_text
                ),
                true,
            )),
            Type::BlockRef => lines.push(self.block_command(
                &base_slot,
                format!(
                    "data modify storage {}:runtime {} set from block $(pos) {}",
                    self.namespace,
                    target.storage_path(),
                    path_text
                ),
                true,
            )),
            _ => {}
        }
    }

    fn compile_array_literal(
        &mut self,
        function: &IrFunction,
        depth: usize,
        values: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {} set value []",
            self.namespace,
            target.storage_path()
        ));
        for value in values {
            let temp = self.new_temp();
            let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
            self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {} append from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                temp_slot.storage_path()
            ));
        }
    }

    fn compile_dict_literal(
        &mut self,
        function: &IrFunction,
        depth: usize,
        entries: &[(String, IrExpr)],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        for (key, value) in entries {
            let temp = self.new_temp();
            let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
            self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                key,
                self.namespace,
                temp_slot.storage_path()
            ));
        }
    }

    fn compile_struct_literal(
        &mut self,
        function: &IrFunction,
        depth: usize,
        fields: &[(String, IrExpr)],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        for (field, value) in fields {
            let temp = self.new_temp();
            let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
            self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                field,
                self.namespace,
                temp_slot.storage_path()
            ));
        }
    }

    fn compile_block_def_spec_string(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let spec_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockDef);
        self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
        let state_fields = self.known_block_builder_state_fields(function, expr);
        if state_fields.is_empty() {
            lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}.id",
                self.namespace,
                target.storage_path(),
                self.namespace,
                spec_slot.storage_path()
            ));
            return;
        }

        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        lines.push(format!(
            "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            spec_slot.storage_path()
        ));
        let mut rendered_states = Vec::new();
        for (index, field) in state_fields.iter().enumerate() {
            let placeholder = format!("s{}", index + 1);
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}.states.{}",
                self.namespace,
                macro_slot.storage_path(),
                placeholder,
                self.namespace,
                spec_slot.storage_path(),
                field
            ));
            rendered_states.push(format!("{}=$({})", field, placeholder));
        }
        let template = format!("$(id)[{}]", rendered_states.join(","));
        lines.push(self.inline_macro_command(
            macro_slot.storage_path(),
            format!(
                "data modify storage {}:runtime {} set value {}",
                self.namespace,
                target.storage_path(),
                quoted(&template)
            ),
        ));
    }

    fn known_block_builder_state_fields(
        &self,
        function: &IrFunction,
        expr: &IrExpr,
    ) -> Vec<String> {
        let name = match &expr.kind {
            IrExprKind::Variable(name) => Some(name.as_str()),
            IrExprKind::Path(path) => match &path.base.kind {
                IrExprKind::Variable(name) if path.base.ty == Type::BlockDef => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        };
        name.and_then(|name| {
            self.block_builder_state_fields
                .get(&function.name)
                .and_then(|fields| fields.get(name))
                .cloned()
        })
        .unwrap_or_default()
    }

    fn render_storage_read_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        base_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) -> RenderedStoragePath {
        self.render_storage_path(
            function,
            depth,
            base_slot.storage_path().to_string(),
            &path.base.ty,
            &path.segments,
            &path.segment_types,
            lines,
        )
    }

    fn render_storage_lvalue_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        lines: &mut Vec<String>,
    ) -> Option<RenderedStoragePath> {
        let IrExprKind::Variable(name) = &path.base.kind else {
            return None;
        };
        let root = string_slot(depth, &function.name, name);
        Some(self.render_storage_path(
            function,
            depth,
            root,
            &path.base.ty,
            &path.segments,
            &path.segment_types,
            lines,
        ))
    }

    fn render_storage_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        root: String,
        root_ty: &Type,
        segments: &[PathSegment],
        segment_types: &[Type],
        lines: &mut Vec<String>,
    ) -> RenderedStoragePath {
        let mut rendered = root;
        let mut current_ty = root_ty.clone();
        let mut macro_storage = None;
        let mut placeholder_index = 0usize;

        for (segment, next_ty) in segments.iter().zip(segment_types.iter()) {
            match (&current_ty, segment) {
                (Type::EntityDef, PathSegment::Field(field))
                | (Type::BlockDef, PathSegment::Field(field))
                | (Type::ItemDef, PathSegment::Field(field))
                | (Type::ItemSlot, PathSegment::Field(field)) => {
                    rendered.push('.');
                    rendered.push_str(field);
                    current_ty = next_ty.clone();
                }
                (Type::Array(element), PathSegment::Index(index)) => {
                    if let crate::ast::ExprKind::Int(value) = &index.kind {
                        rendered.push_str(&format!("[{}]", value));
                    } else {
                        placeholder_index += 1;
                        let name = format!("i{}", placeholder_index);
                        let storage = macro_storage.get_or_insert_with(|| {
                            format!(
                                "frames.d{}.{}.__path{}",
                                depth,
                                sanitize(&function.name),
                                self.new_temp()
                            )
                        });
                        self.compile_expr_to_macro_value(
                            function,
                            depth,
                            index,
                            &Type::Int,
                            storage,
                            &name,
                            lines,
                        );
                        rendered.push_str(&format!("[$({})]", name));
                    }
                    current_ty = *element.clone();
                }
                (Type::Dict(value), PathSegment::Index(index)) => {
                    if let crate::ast::ExprKind::String(key) = &index.kind {
                        rendered.push('.');
                        rendered.push_str(key);
                    } else {
                        placeholder_index += 1;
                        let name = format!("k{}", placeholder_index);
                        let storage = macro_storage.get_or_insert_with(|| {
                            format!(
                                "frames.d{}.{}.__path{}",
                                depth,
                                sanitize(&function.name),
                                self.new_temp()
                            )
                        });
                        self.compile_expr_to_macro_value(
                            function,
                            depth,
                            index,
                            &Type::String,
                            storage,
                            &name,
                            lines,
                        );
                        rendered.push_str(&format!(".$({})", name));
                    }
                    current_ty = *value.clone();
                }
                (_, PathSegment::Field(field)) => {
                    if !rendered.is_empty() {
                        rendered.push('.');
                    }
                    rendered.push_str(field);
                    current_ty = next_ty.clone();
                }
                (Type::Nbt, PathSegment::Index(index)) => {
                    match &index.kind {
                        crate::ast::ExprKind::Int(value) => {
                            rendered.push_str(&format!("[{}]", value));
                        }
                        crate::ast::ExprKind::String(value) => {
                            push_quoted_path_name(&mut rendered, value);
                        }
                        _ => {
                            placeholder_index += 1;
                            let name = format!("n{}", placeholder_index);
                            let storage = macro_storage.get_or_insert_with(|| {
                                format!(
                                    "frames.d{}.{}.__path{}",
                                    depth,
                                    sanitize(&function.name),
                                    self.new_temp()
                                )
                            });
                            let ty = match infer_dynamic_nbt_index_type(function, index) {
                                Some(Type::Int) => Type::Int,
                                _ => Type::String,
                            };
                            self.compile_expr_to_macro_value(
                                function, depth, index, &ty, storage, &name, lines,
                            );
                            match ty {
                                Type::Int => rendered.push_str(&format!("[$({})]", name)),
                                _ => push_quoted_macro_path_name(&mut rendered, &name),
                            }
                        }
                    }
                    current_ty = next_ty.clone();
                }
                (_, PathSegment::Index(index)) => {
                    if let crate::ast::ExprKind::Int(value) = &index.kind {
                        rendered.push_str(&format!("[{}]", value));
                    }
                    current_ty = next_ty.clone();
                }
            }
        }

        RenderedStoragePath {
            path: rendered,
            macro_storage,
        }
    }

    fn render_storage_lvalue_prefix_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        end_exclusive: usize,
        lines: &mut Vec<String>,
    ) -> Option<RenderedStoragePath> {
        let IrExprKind::Variable(name) = &path.base.kind else {
            return None;
        };
        let root = string_slot(depth, &function.name, name);
        Some(self.render_storage_path(
            function,
            depth,
            root,
            &path.base.ty,
            &path.segments[..end_exclusive],
            &path.segment_types[..end_exclusive],
            lines,
        ))
    }

    fn try_compile_storage_index_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        value_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Index(index)) = path.segments.last() else {
            return false;
        };
        let crate::ast::ExprKind::Int(index_value) = &index.kind else {
            return false;
        };
        if *index_value < 0 || path.segments.is_empty() {
            return false;
        }
        let parent_index = path.segments.len() - 1;
        let parent_ty = if parent_index == 0 {
            path.base.ty.clone()
        } else {
            path.segment_types[parent_index - 1].clone()
        };
        if !matches!(parent_ty, Type::Nbt | Type::Array(_)) {
            return false;
        }
        let Some(parent_rendered) =
            self.render_storage_lvalue_prefix_path(function, depth, path, parent_index, lines)
        else {
            return false;
        };
        let element_path = format!("{}[{}]", parent_rendered.path, index_value);
        lines.push(self.storage_path_command(
            format!(
                "execute unless data storage {}:runtime {}[] run data modify storage {}:runtime {} set value []",
                self.namespace,
                parent_rendered.path,
                self.namespace,
                parent_rendered.path
            ),
            parent_rendered.macro_storage.clone(),
        ));
        lines.push(self.storage_path_command(
            format!(
                "execute if data storage {}:runtime {} run data modify storage {}:runtime {} set from storage {}:runtime {}",
                self.namespace,
                element_path,
                self.namespace,
                element_path,
                self.namespace,
                value_slot.storage_path()
            ),
            parent_rendered.macro_storage.clone(),
        ));
        lines.push(self.storage_path_command(
            format!(
                "execute unless data storage {}:runtime {} run data modify storage {}:runtime {} insert {} from storage {}:runtime {}",
                self.namespace,
                element_path,
                self.namespace,
                parent_rendered.path,
                index_value,
                self.namespace,
                value_slot.storage_path()
            ),
            parent_rendered.macro_storage,
        ));
        true
    }

    fn compile_expr_to_macro_value(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &crate::ast::Expr,
        ty: &Type,
        macro_storage: &str,
        name: &str,
        lines: &mut Vec<String>,
    ) {
        let typed_expr = self.lower_macro_path_expr(function, depth, expr, ty);
        let temp = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp, ty);
        self.compile_expr_into_slot(function, depth, &typed_expr, &temp_slot, lines);
        match ty {
            Type::Int | Type::Bool => lines.push(format!(
                "execute store result storage {}:runtime {}.{} int 1 run scoreboard players get {} mcfc",
                self.namespace,
                macro_storage,
                name,
                temp_slot.numeric_name()
            )),
            _ => lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                self.namespace,
                macro_storage,
                name,
                self.namespace,
                temp_slot.storage_path()
            )),
        }
    }

    fn lower_macro_path_expr(
        &self,
        function: &IrFunction,
        depth: usize,
        expr: &crate::ast::Expr,
        ty: &Type,
    ) -> IrExpr {
        let _ = (function, depth);
        match &expr.kind {
            crate::ast::ExprKind::Int(value) => IrExpr {
                ty: Type::Int,
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Int(*value),
            },
            crate::ast::ExprKind::String(value) => IrExpr {
                ty: Type::String,
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::String(value.clone()),
            },
            crate::ast::ExprKind::Bool(value) => IrExpr {
                ty: Type::Bool,
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Bool(*value),
            },
            crate::ast::ExprKind::Variable(name) => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Variable(name.clone()),
            },
            crate::ast::ExprKind::Unary { op, expr } => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Unary {
                    op: *op,
                    expr: Box::new(self.lower_macro_path_expr(function, depth, expr, ty)),
                },
            },
            crate::ast::ExprKind::Binary { op, left, right } => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Binary {
                    op: *op,
                    left: Box::new(self.lower_macro_path_expr(function, depth, left, ty)),
                    right: Box::new(self.lower_macro_path_expr(function, depth, right, ty)),
                },
            },
            crate::ast::ExprKind::Call {
                function: callee,
                args,
            } => {
                let (return_type, params) = self
                    .functions
                    .get(callee)
                    .map(|info| (info.return_type.clone(), info.params.clone()))
                    .unwrap_or((ty.clone(), Vec::new()));
                IrExpr {
                    ty: return_type,
                    ref_kind: RefKind::Unknown,
                    kind: IrExprKind::Call {
                        function: callee.clone(),
                        args: args
                            .iter()
                            .enumerate()
                            .map(|(index, arg)| {
                                let arg_ty = params.get(index).map(|(_, ty)| ty).unwrap_or(ty);
                                self.lower_macro_path_expr(function, depth, arg, arg_ty)
                            })
                            .collect(),
                    },
                }
            }
            _ => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Int(0),
            },
        }
    }

    fn compile_storage_read_from_path(
        &mut self,
        rendered: RenderedStoragePath,
        ty: &Type,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let command = match ty {
            Type::Int | Type::Bool => format!(
                "execute store result score {} mcfc run data get storage {}:runtime {} 1",
                target.numeric_name(),
                self.namespace,
                rendered.path
            ),
            _ => format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                rendered.path
            ),
        };
        lines.push(self.storage_path_command(command, rendered.macro_storage));
    }

    fn storage_path_command(&mut self, command: String, macro_storage: Option<String>) -> String {
        if let Some(storage) = macro_storage {
            let namespace = self.namespace.clone();
            let macro_name = self.ensure_inline_macro(command);
            format!(
                "function {}:{} with storage {}:runtime {}",
                namespace, macro_name, namespace, storage
            )
        } else {
            command
        }
    }

    fn compile_storage_receiver(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        lines: &mut Vec<String>,
    ) -> SlotRef {
        let receiver_name = self.new_temp();
        let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
        self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
        receiver_slot
    }

    fn render_storage_expr_lvalue_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        lines: &mut Vec<String>,
    ) -> Option<RenderedStoragePath> {
        match &expr.kind {
            IrExprKind::Variable(name) => Some(RenderedStoragePath {
                path: string_slot(depth, &function.name, name),
                macro_storage: None,
            }),
            IrExprKind::Path(path) => self.render_storage_lvalue_path(function, depth, path, lines),
            _ => None,
        }
    }

    fn render_dict_key_for_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        key: &IrExpr,
        root: String,
        existing_macro_storage: Option<String>,
        lines: &mut Vec<String>,
    ) -> RenderedStoragePath {
        if let IrExprKind::String(value) = &key.kind {
            return RenderedStoragePath {
                path: format!("{}.{}", root, value),
                macro_storage: existing_macro_storage,
            };
        }

        let macro_storage = existing_macro_storage.unwrap_or_else(|| {
            format!(
                "frames.d{}.{}.__path{}",
                depth,
                sanitize(&function.name),
                self.new_temp()
            )
        });
        let temp = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp, &Type::String);
        self.compile_expr_into_slot(function, depth, key, &temp_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.key set from storage {}:runtime {}",
            self.namespace,
            macro_storage,
            self.namespace,
            temp_slot.storage_path()
        ));
        RenderedStoragePath {
            path: format!("{}.$(key)", root),
            macro_storage: Some(macro_storage),
        }
    }

    fn compile_method_call(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        method: &str,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        match method {
            "as_nbt" => {
                self.compile_value_as_nbt(function, depth, receiver, target, lines);
                return;
            }
            "clear" if receiver.ty == Type::ItemSlot => {
                let slot_handle = self.compile_storage_receiver(function, depth, receiver, lines);
                self.clear_item_slot_handle(function, depth, &slot_handle, lines);
                return;
            }
            "len" => {
                let receiver_slot = self.compile_storage_receiver(function, depth, receiver, lines);
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {}",
                    target.numeric_name(),
                    self.namespace,
                    receiver_slot.storage_path()
                ));
                return;
            }
            "push" => {
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                {
                    if let Some(value) = args.first() {
                        let temp = self.new_temp();
                        let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
                        self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
                        lines.push(self.storage_path_command(
                            format!(
                                "data modify storage {}:runtime {} append from storage {}:runtime {}",
                                self.namespace,
                                rendered.path,
                                self.namespace,
                                temp_slot.storage_path()
                            ),
                            rendered.macro_storage,
                        ));
                    }
                }
                return;
            }
            "pop" => {
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                {
                    let element_ty = match &receiver.ty {
                        Type::Array(element) => element.as_ref(),
                        _ => &Type::Nbt,
                    };
                    self.compile_storage_read_from_path(
                        RenderedStoragePath {
                            path: format!("{}[-1]", rendered.path),
                            macro_storage: rendered.macro_storage.clone(),
                        },
                        element_ty,
                        target,
                        lines,
                    );
                    lines.push(self.storage_path_command(
                        format!(
                            "data remove storage {}:runtime {}[-1]",
                            self.namespace, rendered.path
                        ),
                        rendered.macro_storage,
                    ));
                }
                return;
            }
            "remove_at" => {
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                {
                    let element_ty = match &receiver.ty {
                        Type::Array(element) => element.as_ref(),
                        _ => &Type::Nbt,
                    };
                    if let Some(index) = args.first() {
                        let macro_storage = rendered.macro_storage.clone().unwrap_or_else(|| {
                            format!(
                                "frames.d{}.{}.__path{}",
                                depth,
                                sanitize(&function.name),
                                self.new_temp()
                            )
                        });
                        let index_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                        self.compile_expr_into_slot(function, depth, index, &index_slot, lines);
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_storage,
                            index_slot.numeric_name()
                        ));
                        self.compile_storage_read_from_path(
                            RenderedStoragePath {
                                path: format!("{}[$(index)]", rendered.path),
                                macro_storage: Some(macro_storage.clone()),
                            },
                            element_ty,
                            target,
                            lines,
                        );
                        lines.push(self.storage_path_command(
                            format!(
                                "data remove storage {}:runtime {}[$(index)]",
                                self.namespace, rendered.path
                            ),
                            Some(macro_storage),
                        ));
                    }
                }
                return;
            }
            "has" => {
                let receiver_slot = self.compile_storage_receiver(function, depth, receiver, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(key) = args.first() {
                    let key_rendered = self.render_dict_key_for_method(
                        function,
                        depth,
                        key,
                        receiver_slot.storage_path().to_string(),
                        None,
                        lines,
                    );
                    lines.push(self.storage_path_command(
                        format!(
                            "execute if data storage {}:runtime {} run scoreboard players set {} mcfc 1",
                            self.namespace,
                            key_rendered.path,
                            target.numeric_name()
                        ),
                        key_rendered.macro_storage,
                    ));
                }
                return;
            }
            "remove" => {
                if receiver.ty == Type::Bossbar {
                    let receiver_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
                    self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                    let macro_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                        self.namespace,
                        macro_slot.storage_path(),
                        self.namespace,
                        receiver_slot.storage_path()
                    ));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        "bossbar remove $(id)".to_string(),
                    ));
                    return;
                }
                if matches!(receiver.ty, Type::Array(_)) {
                    if let Some(rendered) =
                        self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                    {
                        let element_ty = match &receiver.ty {
                            Type::Array(element) => element.as_ref(),
                            _ => &Type::Nbt,
                        };
                        if let Some(index) = args.first() {
                            let macro_storage =
                                rendered.macro_storage.clone().unwrap_or_else(|| {
                                    format!(
                                        "frames.d{}.{}.__path{}",
                                        depth,
                                        sanitize(&function.name),
                                        self.new_temp()
                                    )
                                });
                            let index_slot =
                                local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                            self.compile_expr_into_slot(function, depth, index, &index_slot, lines);
                            lines.push(format!(
                                "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                                self.namespace,
                                macro_storage,
                                index_slot.numeric_name()
                            ));
                            self.compile_storage_read_from_path(
                                RenderedStoragePath {
                                    path: format!("{}[$(index)]", rendered.path),
                                    macro_storage: Some(macro_storage.clone()),
                                },
                                element_ty,
                                target,
                                lines,
                            );
                            lines.push(self.storage_path_command(
                                format!(
                                    "data remove storage {}:runtime {}[$(index)]",
                                    self.namespace, rendered.path
                                ),
                                Some(macro_storage),
                            ));
                        }
                    }
                    return;
                }
                if let Some(key) = args.first() {
                    if let Some(receiver_path) =
                        self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                    {
                        let key_rendered = self.render_dict_key_for_method(
                            function,
                            depth,
                            key,
                            receiver_path.path,
                            receiver_path.macro_storage,
                            lines,
                        );
                        lines.push(self.storage_path_command(
                            format!(
                                "data remove storage {}:runtime {}",
                                self.namespace, key_rendered.path
                            ),
                            key_rendered.macro_storage,
                        ));
                    }
                }
                return;
            }
            "teleport" | "damage" | "heal" | "give" | "clear" | "loot_give" | "tellraw"
            | "title" | "actionbar" | "debug_entity" => {
                if method == "give" && args.len() == 1 && args[0].ty == Type::ItemDef {
                    self.compile_entity_give_item_def(function, depth, receiver, &args[0], lines);
                    return;
                }
                let mut synthetic = Vec::with_capacity(args.len() + 1);
                synthetic.push(receiver.clone());
                synthetic.extend(args.iter().cloned());
                self.compile_builtin_call(function, depth, method, &synthetic, target, lines);
                return;
            }
            "playsound" => {
                if args.len() >= 2 {
                    let synthetic = vec![args[0].clone(), args[1].clone(), receiver.clone()];
                    self.compile_builtin_call(
                        function,
                        depth,
                        "playsound",
                        &synthetic,
                        target,
                        lines,
                    );
                }
                return;
            }
            "stopsound" => {
                if args.len() >= 2 {
                    let synthetic = vec![receiver.clone(), args[0].clone(), args[1].clone()];
                    self.compile_builtin_call(
                        function,
                        depth,
                        "stopsound",
                        &synthetic,
                        target,
                        lines,
                    );
                }
                return;
            }
            "summon" => {
                self.compile_block_summon_method(function, depth, receiver, args, target, lines);
                return;
            }
            "spawn_item" => {
                self.compile_block_spawn_item_method(
                    function, depth, receiver, args, target, lines,
                );
                return;
            }
            "loot_insert" | "loot_spawn" | "setblock" | "fill" | "debug_marker" => {
                let mut synthetic = Vec::with_capacity(args.len() + 1);
                synthetic.push(receiver.clone());
                synthetic.extend(args.iter().cloned());
                self.compile_builtin_call(function, depth, method, &synthetic, target, lines);
                return;
            }
            "is" => {
                let receiver_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(arg) = args.first() {
                    let block_id_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &block_id_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        block_id_slot.storage_path()
                    ));
                    lines.push(self.block_command(
                        &receiver_slot,
                        format!(
                            "execute if block $(pos) $(block) run scoreboard players set {} mcfc 1",
                            target.numeric_name()
                        ),
                        true,
                    ));
                }
                return;
            }
            "particle" => {
                if let Some(name) = args.first() {
                    let mut synthetic = Vec::with_capacity(args.len() + 1);
                    synthetic.push(name.clone());
                    synthetic.push(receiver.clone());
                    synthetic.extend(args.iter().skip(1).cloned());
                    self.compile_builtin_call(
                        function, depth, "particle", &synthetic, target, lines,
                    );
                }
                return;
            }
            "add_tag" | "remove_tag" => {
                let receiver_name = self.new_temp();
                let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                if let Some(arg) = args.first() {
                    let tag_name = self.new_temp();
                    let tag_slot = local_slot(depth, &function.name, &tag_name, &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &tag_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.tag set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        tag_slot.storage_path()
                    ));
                    lines.push(self.query_command(
                        &receiver_slot,
                        format!(
                            "tag $(selector) {} $(tag)",
                            if method == "add_tag" { "add" } else { "remove" }
                        ),
                        true,
                    ));
                }
                return;
            }
            "has_tag" => {
                let receiver_name = self.new_temp();
                let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(arg) = args.first() {
                    let tag_name = self.new_temp();
                    let tag_slot = local_slot(depth, &function.name, &tag_name, &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &tag_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.tag set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        tag_slot.storage_path()
                    ));
                    lines.push(self.query_command(
                        &receiver_slot,
                        format!(
                            "execute as $(selector) if entity @s[tag=$(tag)] run scoreboard players set {} mcfc 1",
                            target.numeric_name()
                        ),
                        true,
                    ));
                }
                return;
            }
            _ => {}
        }
        if method != "effect" {
            return;
        }
        let receiver_name = self.new_temp();
        let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
        self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);

        let effect_name = self.new_temp();
        let duration_name = self.new_temp();
        let amplifier_name = self.new_temp();
        let effect_slot = local_slot(depth, &function.name, &effect_name, &Type::String);
        let duration_slot = local_slot(depth, &function.name, &duration_name, &Type::Int);
        let amplifier_slot = local_slot(depth, &function.name, &amplifier_name, &Type::Int);
        if let Some(arg) = args.first() {
            self.compile_expr_into_slot(function, depth, arg, &effect_slot, lines);
        }
        if let Some(arg) = args.get(1) {
            self.compile_expr_into_slot(function, depth, arg, &duration_slot, lines);
        }
        if let Some(arg) = args.get(2) {
            self.compile_expr_into_slot(function, depth, arg, &amplifier_slot, lines);
        }

        let composed_name = self.new_temp();
        let composed_slot = local_slot(depth, &function.name, &composed_name, &Type::EntityRef);
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            composed_slot.storage_path(),
            self.namespace,
            receiver_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            composed_slot.storage_path(),
            self.namespace,
            receiver_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.effect set from storage {}:runtime {}",
            self.namespace,
            composed_slot.storage_path(),
            self.namespace,
            effect_slot.storage_path()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.duration int 1 run scoreboard players get {} mcfc",
            self.namespace,
            composed_slot.storage_path(),
            duration_slot.numeric_name()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.amplifier int 1 run scoreboard players get {} mcfc",
            self.namespace,
            composed_slot.storage_path(),
            amplifier_slot.numeric_name()
        ));
        lines.push(self.query_command(
            &composed_slot,
            "effect give $(selector) $(effect) $(duration) $(amplifier) true".to_string(),
            true,
        ));
    }

    fn compile_entity_give_item_def(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        item: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let target_slot = self.compile_storage_receiver(function, depth, receiver, lines);
        let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::ItemDef);
        self.compile_expr_into_slot(function, depth, item, &item_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.item set from storage {}:runtime {}.id",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.item_name",
            self.namespace,
            target_slot.storage_path()
        ));
        lines.push(format!(
            "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {}.item_name set from storage {}:runtime {}.nbt.display.Name",
            self.namespace,
            item_slot.storage_path(),
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set from storage {}:runtime {}.count",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        let named_give = self.query_command(
            &target_slot,
            "give $(selector) $(item)[minecraft:custom_name='\"$(item_name)\"',minecraft:custom_data=$(data)] $(count)".to_string(),
            true,
        );
        let plain_give = self.query_command(
            &target_slot,
            "give $(selector) $(item)[minecraft:custom_data=$(data)] $(count)".to_string(),
            true,
        );
        lines.push(format!(
            "execute if data storage {}:runtime {}.item_name run {}",
            self.namespace,
            target_slot.storage_path(),
            named_give
        ));
        lines.push(format!(
            "execute unless data storage {}:runtime {}.item_name run {}",
            self.namespace,
            target_slot.storage_path(),
            plain_give
        ));
    }

    fn compile_block_summon_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let pos_slot = self.compile_storage_receiver(function, depth, receiver, lines);
        let payload_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let entity_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        if args.first().is_some_and(|arg| arg.ty == Type::EntityDef) {
            let spec_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::EntityDef);
            self.compile_expr_into_slot(function, depth, &args[0], &spec_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}.id",
                self.namespace,
                entity_slot.storage_path(),
                self.namespace,
                spec_slot.storage_path()
            ));
            lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}.nbt",
                self.namespace,
                payload_slot.storage_path(),
                self.namespace,
                spec_slot.storage_path()
            ));
            lines.push(format!(
                "execute unless data storage {}:runtime {} run data modify storage {}:runtime {} set value {{}}",
                self.namespace,
                payload_slot.storage_path(),
                self.namespace,
                payload_slot.storage_path()
            ));
        } else {
            if let Some(arg) = args.first() {
                self.compile_expr_into_slot(function, depth, arg, &entity_slot, lines);
            }
            if let Some(arg) = args.get(1) {
                self.compile_value_as_nbt(function, depth, arg, &payload_slot, lines);
            } else {
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    payload_slot.storage_path()
                ));
            }
        }
        self.compile_summon_from_position_slot(
            function,
            depth,
            &pos_slot,
            &entity_slot,
            &payload_slot,
            target,
            lines,
        );
    }

    fn compile_block_spawn_item_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let pos_slot = self.compile_storage_receiver(function, depth, receiver, lines);
        let entity_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let payload_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        lines.push(format!(
            "data modify storage {}:runtime {} set value \"minecraft:item\"",
            self.namespace,
            entity_slot.storage_path()
        ));
        if let Some(item) = args.first() {
            self.compile_value_as_nbt(function, depth, item, &item_slot, lines);
        } else {
            lines.push(format!(
                "data modify storage {}:runtime {} set value {{id:\"minecraft:air\",Count:0b}}",
                self.namespace,
                item_slot.storage_path()
            ));
        }
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{}}",
            self.namespace,
            payload_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.Item set from storage {}:runtime {}",
            self.namespace,
            payload_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        self.compile_summon_from_position_slot(
            function,
            depth,
            &pos_slot,
            &entity_slot,
            &payload_slot,
            target,
            lines,
        );
    }

    fn compile_summon_from_position_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        pos_slot: &SlotRef,
        entity_slot: &SlotRef,
        payload_slot: &SlotRef,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let summon_target = if target.storage_path() == "__void" {
            local_slot(depth, &function.name, &self.new_temp(), &Type::EntityRef)
        } else {
            target.clone()
        };
        let capture_tag = format!("mcfc_summon_capture_{}", self.new_temp());
        let ref_tag = format!("mcfc_summon_ref_{}", self.new_temp());
        lines.push(format!("tag @e[tag={}] remove {}", ref_tag, ref_tag));
        lines.push(format!(
            "tag @e[tag={}] remove {}",
            capture_tag, capture_tag
        ));
        lines.push(format!(
            "execute unless data storage {}:runtime {}.Tags[] run data modify storage {}:runtime {}.Tags set value []",
            self.namespace,
            payload_slot.storage_path(),
            self.namespace,
            payload_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.Tags append value {}",
            self.namespace,
            payload_slot.storage_path(),
            quoted(&capture_tag)
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.Tags append value {}",
            self.namespace,
            payload_slot.storage_path(),
            quoted(&ref_tag)
        ));
        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            pos_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.pos set from storage {}:runtime {}.pos",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            pos_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.entity set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            entity_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.data set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            payload_slot.storage_path()
        ));
        lines.push(self.inline_macro_command(
            macro_slot.storage_path(),
            "$(prefix)summon $(entity) $(pos) $(data)".to_string(),
        ));
        self.write_query_slot(
            &summon_target,
            "",
            &format!("@e[tag={},sort=nearest,limit=1]", ref_tag),
            lines,
        );
        lines.push(self.query_command(
            &summon_target,
            format!("tag $(selector) remove {}", capture_tag),
            true,
        ));
    }

    fn compile_builtin_call(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        match callee {
            "random" => {
                if args.is_empty() {
                    lines.push(format!(
                        "execute store result score {} mcfc run random value 0..2147483647",
                        target.numeric_name()
                    ));
                    return true;
                }

                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let min_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                let max_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                match args {
                    [max] => {
                        lines.push(format!(
                            "scoreboard players set {} mcfc 0",
                            min_slot.numeric_name()
                        ));
                        self.compile_expr_into_slot(function, depth, max, &max_slot, lines);
                    }
                    [min, max] => {
                        self.compile_expr_into_slot(function, depth, min, &min_slot, lines);
                        self.compile_expr_into_slot(function, depth, max, &max_slot, lines);
                    }
                    _ => return true,
                }
                lines.push(format!(
                    "execute store result storage {}:runtime {}.min int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    min_slot.numeric_name()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.max int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    max_slot.numeric_name()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!(
                        "execute store result score {} mcfc run random value $(min)..$(max)",
                        target.numeric_name()
                    ),
                ));
                true
            }
            "bossbar" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    target.storage_path()
                ));
                if let IrExprKind::String(text) = &args[1].kind {
                    let component = selector_text_components(text).unwrap_or_else(|| quoted(text));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        format!("bossbar add $(id) {}", component),
                    ));
                    return true;
                }
                if args[1].ty == Type::TextDef {
                    let name_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
                    self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                        self.namespace,
                        macro_slot.storage_path(),
                        self.namespace,
                        name_slot.storage_path()
                    ));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        "bossbar add $(id) $(name)".to_string(),
                    ));
                    return true;
                }
                let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    name_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "bossbar add $(id) [\"$(name)\"]".to_string(),
                ));
                true
            }
            "entity" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                true
            }
            "block_type" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.states set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                true
            }
            "item" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.count set value 1",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                true
            }
            "text" => {
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                if let Some(arg) = args.first() {
                    let text_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &text_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.text set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        text_slot.storage_path()
                    ));
                }
                true
            }
            "summon" => {
                let summon_target = if target.storage_path() == "__void" {
                    local_slot(depth, &function.name, &self.new_temp(), &Type::EntityRef)
                } else {
                    target.clone()
                };
                let payload_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let entity_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                if args.first().is_some_and(|arg| arg.ty == Type::EntityDef) {
                    let spec_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::EntityDef);
                    self.compile_expr_into_slot(function, depth, &args[0], &spec_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}.id",
                        self.namespace,
                        entity_slot.storage_path(),
                        self.namespace,
                        spec_slot.storage_path()
                    ));
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}.nbt",
                        self.namespace,
                        payload_slot.storage_path(),
                        self.namespace,
                        spec_slot.storage_path()
                    ));
                } else {
                    if let Some(arg) = args.first() {
                        self.compile_expr_into_slot(function, depth, arg, &entity_slot, lines);
                    }
                    if let Some(arg) = args.get(1) {
                        self.compile_value_as_nbt(function, depth, arg, &payload_slot, lines);
                    } else {
                        lines.push(format!(
                            "data modify storage {}:runtime {} set value {{}}",
                            self.namespace,
                            payload_slot.storage_path()
                        ));
                    }
                }
                if args.first().is_some_and(|arg| arg.ty == Type::EntityDef) {
                    lines.push(format!(
                        "execute unless data storage {}:runtime {} run data modify storage {}:runtime {} set value {{}}",
                        self.namespace,
                        payload_slot.storage_path(),
                        self.namespace,
                        payload_slot.storage_path()
                    ));
                } else {
                    // handled above for the string overload
                }
                let capture_tag = format!("mcfc_summon_capture_{}", self.new_temp());
                let ref_tag = format!("mcfc_summon_ref_{}", self.new_temp());
                lines.push(format!("tag @e[tag={}] remove {}", ref_tag, ref_tag));
                lines.push(format!(
                    "tag @e[tag={}] remove {}",
                    capture_tag, capture_tag
                ));
                lines.push(format!(
                    "execute unless data storage {}:runtime {}.Tags[] run data modify storage {}:runtime {}.Tags set value []",
                    self.namespace,
                    payload_slot.storage_path(),
                    self.namespace,
                    payload_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.Tags append value {}",
                    self.namespace,
                    payload_slot.storage_path(),
                    quoted(&capture_tag)
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.Tags append value {}",
                    self.namespace,
                    payload_slot.storage_path(),
                    quoted(&ref_tag)
                ));
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.entity set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    entity_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.data set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    payload_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "summon $(entity) ~ ~ ~ $(data)".to_string(),
                ));
                self.write_query_slot(
                    &summon_target,
                    "",
                    &format!("@e[tag={},sort=nearest,limit=1]", ref_tag),
                    lines,
                );
                lines.push(self.query_command(
                    &summon_target,
                    format!("tag $(selector) remove {}", capture_tag),
                    true,
                ));
                true
            }
            "teleport" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let destination_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
                self.compile_expr_into_slot(function, depth, &args[1], &destination_slot, lines);
                match args[1].ty {
                    Type::EntityRef | Type::PlayerRef | Type::EntitySet => {
                        lines.push(format!(
                            "data modify storage {}:runtime {}.dest set from storage {}:runtime {}.selector",
                            self.namespace,
                            target_slot.storage_path(),
                            self.namespace,
                            destination_slot.storage_path()
                        ));
                        lines.push(self.query_command(
                            &target_slot,
                            "teleport $(selector) $(dest)".to_string(),
                            true,
                        ));
                    }
                    Type::BlockRef => {
                        lines.push(format!(
                            "data modify storage {}:runtime {}.dest set from storage {}:runtime {}.pos",
                            self.namespace,
                            target_slot.storage_path(),
                            self.namespace,
                            destination_slot.storage_path()
                        ));
                        lines.push(self.query_command(
                            &target_slot,
                            "teleport $(selector) $(dest)".to_string(),
                            true,
                        ));
                    }
                    _ => {}
                }
                true
            }
            "damage" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let amount_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, &args[1], &amount_slot, lines);
                lines.push(format!(
                    "execute store result storage {}:runtime {}.amount int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_slot.storage_path(),
                    amount_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "damage $(selector) $(amount)".to_string(),
                    true,
                ));
                true
            }
            "heal" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let amount_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, &args[1], &amount_slot, lines);
                let health_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                lines.push(self.query_command(
                    &target_slot,
                    format!(
                        "execute store result score {} mcfc run data get entity $(selector) Health 1",
                        health_slot.numeric_name()
                    ),
                    true,
                ));
                lines.push(format!(
                    "scoreboard players operation {} mcfc += {} mcfc",
                    health_slot.numeric_name(),
                    amount_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    format!(
                        "execute store result entity $(selector) Health float 1 run scoreboard players get {} mcfc",
                        health_slot.numeric_name()
                    ),
                    true,
                ));
                true
            }
            "give" | "clear" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &item_slot, lines);
                let count_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, &args[2], &count_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.item set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    item_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_slot.storage_path(),
                    count_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    format!("{} $(selector) $(item) $(count)", callee),
                    true,
                ));
                true
            }
            "loot_give" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let table_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &table_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.table set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    table_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "loot give $(selector) loot $(table)".to_string(),
                    true,
                ));
                true
            }
            "loot_insert" | "loot_spawn" | "setblock" => {
                let block_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &block_slot, lines);
                if callee == "setblock" && args[1].ty == Type::BlockDef {
                    let block_string_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    let block_data_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                    self.compile_block_def_spec_string(
                        function,
                        depth,
                        &args[1],
                        &block_string_slot,
                        lines,
                    );
                    lines.push(format!(
                        "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                        self.namespace,
                        block_slot.storage_path(),
                        self.namespace,
                        block_string_slot.storage_path()
                    ));
                    lines.push(self.block_command(
                        &block_slot,
                        "setblock $(pos) $(block)".to_string(),
                        true,
                    ));
                    self.compile_expr_into_slot(function, depth, &args[1], &block_data_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
                        self.namespace,
                        block_slot.storage_path(),
                        self.namespace,
                        block_data_slot.storage_path()
                    ));
                    lines.push(self.block_command(
                        &block_slot,
                        "data merge block $(pos) $(data)".to_string(),
                        true,
                    ));
                } else {
                    let value_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, &args[1], &value_slot, lines);
                    let field = if callee == "setblock" {
                        "block"
                    } else {
                        "table"
                    };
                    lines.push(format!(
                        "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                        self.namespace,
                        block_slot.storage_path(),
                        field,
                        self.namespace,
                        value_slot.storage_path()
                    ));
                    let command = match callee {
                        "loot_insert" => "loot insert $(pos) loot $(table)".to_string(),
                        "loot_spawn" => "loot spawn $(pos) loot $(table)".to_string(),
                        _ => "setblock $(pos) $(block)".to_string(),
                    };
                    lines.push(self.block_command(&block_slot, command, true));
                }
                true
            }
            "fill" => {
                let from_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                let to_slot = local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
                let block_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &from_slot, lines);
                self.compile_expr_into_slot(function, depth, &args[1], &to_slot, lines);
                if args[2].ty == Type::BlockDef {
                    self.compile_block_def_spec_string(
                        function,
                        depth,
                        &args[2],
                        &block_slot,
                        lines,
                    );
                } else {
                    self.compile_expr_into_slot(function, depth, &args[2], &block_slot, lines);
                }
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.from set from storage {}:runtime {}.pos",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    from_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.to set from storage {}:runtime {}.pos",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    to_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    block_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "fill $(from) $(to) $(block)".to_string(),
                ));
                true
            }
            "tellraw" | "title" | "actionbar" => {
                self.compile_display_builtin(function, depth, callee, args, lines);
                true
            }
            "debug" => {
                self.compile_debug_builtin(function, depth, args, lines);
                true
            }
            "debug_marker" => {
                self.compile_debug_marker_builtin(function, depth, args, lines);
                true
            }
            "debug_entity" => {
                self.compile_debug_entity_builtin(function, depth, args, lines);
                true
            }
            "bossbar_add" | "bossbar_name" => {
                self.compile_bossbar_text_builtin(function, depth, callee, args, lines);
                true
            }
            "bossbar_remove" | "bossbar_value" | "bossbar_max" | "bossbar_visible" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                let command = match callee {
                    "bossbar_remove" => "bossbar remove $(id)".to_string(),
                    "bossbar_value" => {
                        let value_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                        self.compile_expr_into_slot(function, depth, &args[1], &value_slot, lines);
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.value int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_slot.storage_path(),
                            value_slot.numeric_name()
                        ));
                        "bossbar set $(id) value $(value)".to_string()
                    }
                    "bossbar_max" => {
                        let value_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                        self.compile_expr_into_slot(function, depth, &args[1], &value_slot, lines);
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.value int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_slot.storage_path(),
                            value_slot.numeric_name()
                        ));
                        "bossbar set $(id) max $(value)".to_string()
                    }
                    _ => {
                        let visible_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
                        self.compile_expr_into_slot(
                            function,
                            depth,
                            &args[1],
                            &visible_slot,
                            lines,
                        );
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.visible int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_slot.storage_path(),
                            visible_slot.numeric_name()
                        ));
                        "bossbar set $(id) visible $(visible)".to_string()
                    }
                };
                lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
                true
            }
            "bossbar_players" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
                self.compile_expr_into_slot(function, depth, &args[1], &target_slot, lines);
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "bossbar set $(id) players $(selector)".to_string(),
                    true,
                ));
                true
            }
            "playsound" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[2].ty);
                self.compile_expr_into_slot(function, depth, &args[2], &target_slot, lines);
                let sound_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                let category_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &sound_slot, lines);
                self.compile_expr_into_slot(function, depth, &args[1], &category_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.sound set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    sound_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.category set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    category_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "playsound $(sound) $(category) $(selector) ~ ~ ~ 1 1 1".to_string(),
                    true,
                ));
                true
            }
            "stopsound" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let category_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                let sound_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &category_slot, lines);
                self.compile_expr_into_slot(function, depth, &args[2], &sound_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.category set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    category_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.sound set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    sound_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "stopsound $(selector) $(category) $(sound)".to_string(),
                    true,
                ));
                true
            }
            "particle" => {
                self.compile_particle_builtin(function, depth, args, lines);
                true
            }
            _ => false,
        }
    }

    fn compile_display_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
        self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
        if let IrExprKind::String(text) = &args[1].kind {
            let component = display_text_components(text).unwrap_or_else(|| quoted(text));
            let command = match callee {
                "tellraw" => format!("tellraw $(selector) {}", component),
                "title" => format!("title $(selector) title {}", component),
                _ => format!("title $(selector) actionbar {}", component),
            };
            lines.push(self.query_command(&target_slot, command, true));
            return;
        }
        if args[1].ty == Type::TextDef {
            let message_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
            self.compile_expr_into_slot(function, depth, &args[1], &message_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.message set from storage {}:runtime {}",
                self.namespace,
                target_slot.storage_path(),
                self.namespace,
                message_slot.storage_path()
            ));
            let command = match callee {
                "tellraw" => "tellraw $(selector) $(message)".to_string(),
                "title" => "title $(selector) title $(message)".to_string(),
                _ => "title $(selector) actionbar $(message)".to_string(),
            };
            lines.push(self.query_command(&target_slot, command, true));
            return;
        }
        if let IrExprKind::InterpolatedString {
            template,
            placeholders,
        } = &args[1].kind
        {
            self.compile_interpolated_display_builtin(
                function,
                depth,
                callee,
                &target_slot,
                template,
                placeholders,
                lines,
            );
            return;
        }
        let message_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &message_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.message set from storage {}:runtime {}",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            message_slot.storage_path()
        ));
        let command = match callee {
            "tellraw" => "tellraw $(selector) [\"$(message)\"]".to_string(),
            "title" => "title $(selector) title [\"$(message)\"]".to_string(),
            _ => "title $(selector) actionbar [\"$(message)\"]".to_string(),
        };
        lines.push(self.query_command(&target_slot, command, true));
    }

    fn compile_interpolated_display_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        target_slot: &SlotRef,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        lines: &mut Vec<String>,
    ) {
        self.macro_counter += 1;
        let macro_id = self.macro_counter;
        let storage_base = macro_storage_base(depth, &function.name, macro_id);
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            storage_base,
            self.namespace,
            target_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            storage_base,
            self.namespace,
            target_slot.storage_path()
        ));
        self.write_macro_placeholder_values(function, depth, &storage_base, placeholders, lines);

        let rewritten = rewrite_macro_template(template, placeholders);
        let component = display_text_components(&rewritten)
            .unwrap_or_else(|| format!("[{}]", quoted(&rewritten)));
        let command = match callee {
            "tellraw" => format!("$(prefix)tellraw $(selector) {}", component),
            "title" => format!("$(prefix)title $(selector) title {}", component),
            _ => format!("$(prefix)title $(selector) actionbar {}", component),
        };
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(command);
        lines.push(format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage_base
        ));
    }

    fn compile_debug_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        if let IrExprKind::String(message) = &args[0].kind {
            lines.push(format!(
                "tellraw @a [{{\"text\":\"[MCFC debug] \",\"color\":\"gold\"}},{{\"text\":{},\"color\":\"white\"}}]",
                quoted(message)
            ));
            return;
        }

        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let message_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[0], &message_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.message set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            message_slot.storage_path()
        ));
        lines.push(self.inline_macro_command(
            macro_slot.storage_path(),
            "tellraw @a [{\"text\":\"[MCFC debug] \",\"color\":\"gold\"},{\"text\":\"$(message)\",\"color\":\"white\"}]".to_string(),
        ));
    }

    fn compile_debug_marker_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
        self.compile_expr_into_slot(function, depth, &args[0], &pos_slot, lines);
        let label_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &label_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.label set from storage {}:runtime {}",
            self.namespace,
            pos_slot.storage_path(),
            self.namespace,
            label_slot.storage_path()
        ));
        lines.push(self.block_command(
            &pos_slot,
            "tellraw @a [{\"text\":\"[MCFC marker] \",\"color\":\"aqua\"},{\"text\":\"$(label) at $(pos)\",\"color\":\"white\"}]".to_string(),
            true,
        ));
        lines.push(self.block_command(
            &pos_slot,
            "particle minecraft:happy_villager $(pos) 0.35 0.75 0.35 0 40 force @a".to_string(),
            true,
        ));
        lines.push(self.block_command(
            &pos_slot,
            "playsound minecraft:block.note_block.pling master @a $(pos) 1 1.6 1".to_string(),
            true,
        ));

        if let Some(block) = args.get(2) {
            let block_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
            self.compile_expr_into_slot(function, depth, block, &block_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                self.namespace,
                pos_slot.storage_path(),
                self.namespace,
                block_slot.storage_path()
            ));
            lines.push(self.block_command(
                &pos_slot,
                "setblock $(pos) $(block) replace".to_string(),
                true,
            ));
        }
    }

    fn compile_debug_entity_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
        self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
        let label_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &label_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.label set from storage {}:runtime {}",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            label_slot.storage_path()
        ));
        lines.push(self.query_command(
            &target_slot,
            "execute if entity $(selector) run tellraw @a [{\"text\":\"[MCFC entity] found \",\"color\":\"green\"},{\"text\":\"$(label) \"},{\"selector\":\"$(selector)\"}]".to_string(),
            true,
        ));
        lines.push(self.query_command(
            &target_slot,
            "execute unless entity $(selector) run tellraw @a [{\"text\":\"[MCFC entity] missing \",\"color\":\"red\"},{\"text\":\"$(label)\"}]".to_string(),
            true,
        ));
        lines.push(self.query_command(
            &target_slot,
            "execute if entity $(selector) run effect give $(selector) minecraft:glowing 3 0 true".to_string(),
            true,
        ));
    }

    fn compile_bossbar_text_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            id_slot.storage_path()
        ));
        if let IrExprKind::String(text) = &args[1].kind {
            let component = selector_text_components(text).unwrap_or_else(|| quoted(text));
            let command = if callee == "bossbar_add" {
                format!("bossbar add $(id) {}", component)
            } else {
                format!("bossbar set $(id) name {}", component)
            };
            lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
            return;
        }
        if args[1].ty == Type::TextDef {
            let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
            self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                self.namespace,
                macro_slot.storage_path(),
                self.namespace,
                name_slot.storage_path()
            ));
            let command = if callee == "bossbar_add" {
                "bossbar add $(id) $(name)".to_string()
            } else {
                "bossbar set $(id) name $(name)".to_string()
            };
            lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
            return;
        }
        let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            name_slot.storage_path()
        ));
        let command = if callee == "bossbar_add" {
            "bossbar add $(id) [\"$(name)\"]".to_string()
        } else {
            "bossbar set $(id) name [\"$(name)\"]".to_string()
        };
        lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
    }

    fn compile_particle_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
        self.compile_expr_into_slot(function, depth, &args[1], &pos_slot, lines);
        let particle_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[0], &particle_slot, lines);
        let count_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
        if let Some(arg) = args.get(2) {
            self.compile_expr_into_slot(function, depth, arg, &count_slot, lines);
        } else {
            lines.push(format!(
                "scoreboard players set {} mcfc 1",
                count_slot.numeric_name()
            ));
        }
        if let Some(viewers) = args.get(3) {
            let viewer_slot = local_slot(depth, &function.name, &self.new_temp(), &viewers.ty);
            self.compile_expr_into_slot(function, depth, viewers, &viewer_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.particle set from storage {}:runtime {}",
                self.namespace,
                viewer_slot.storage_path(),
                self.namespace,
                particle_slot.storage_path()
            ));
            lines.push(format!(
                "data modify storage {}:runtime {}.pos set from storage {}:runtime {}.pos",
                self.namespace,
                viewer_slot.storage_path(),
                self.namespace,
                pos_slot.storage_path()
            ));
            lines.push(format!(
                "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
                self.namespace,
                viewer_slot.storage_path(),
                count_slot.numeric_name()
            ));
            lines.push(self.query_command(
                &viewer_slot,
                "particle $(particle) $(pos) 0 0 0 0 $(count) force $(selector)".to_string(),
                true,
            ));
            return;
        }
        lines.push(format!(
            "data modify storage {}:runtime {}.particle set from storage {}:runtime {}",
            self.namespace,
            pos_slot.storage_path(),
            self.namespace,
            particle_slot.storage_path()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
            self.namespace,
            pos_slot.storage_path(),
            count_slot.numeric_name()
        ));
        lines.push(self.block_command(
            &pos_slot,
            "particle $(particle) $(pos) 0 0 0 0 $(count) force".to_string(),
            true,
        ));
    }

    fn try_compile_player_path_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        value: &IrExpr,
        value_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Field(first)) = path.segments.first() else {
            return false;
        };
        match first.as_str() {
            "nbt" if path.base.ref_kind == RefKind::Player => true,
            "state" => {
                if path.segments.len() == 1 {
                    return false;
                }
                let objective = state_objective(path.base.ref_kind, &path.segments[1..]);
                let temp_name = self.new_temp();
                let temp_slot = local_slot(depth, &function.name, &temp_name, &Type::Int);
                self.compile_expr_into_slot(function, depth, value, &temp_slot, lines);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "scoreboard players operation $(selector) {} = {} mcfc",
                        objective,
                        temp_slot.numeric_name()
                    ),
                    true,
                ));
                true
            }
            "tags" => {
                if path.base.ref_kind != RefKind::Player {
                    return false;
                }
                let tag = render_path_segments(&path.segments[1..]);
                let temp_name = self.new_temp();
                let temp_slot = local_slot(depth, &function.name, &temp_name, &Type::Bool);
                self.compile_expr_into_slot(function, depth, value, &temp_slot, lines);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute if score {} mcfc matches 1 run tag $(selector) add {}",
                        temp_slot.numeric_name(),
                        tag
                    ),
                    true,
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute if score {} mcfc matches 0 run tag $(selector) remove {}",
                        temp_slot.numeric_name(),
                        tag
                    ),
                    true,
                ));
                true
            }
            "team" => {
                lines.push(format!(
                    "data modify storage {}:runtime {}.team set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    value_slot.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    "team join $(team) $(selector)".to_string(),
                    true,
                ));
                true
            }
            "inventory" | "hotbar" => {
                let Some((namespace, index)) = player_item_slot_index_from_path(path) else {
                    return false;
                };
                let slot_handle =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                self.load_player_item_slot(
                    function,
                    depth,
                    base_slot,
                    namespace,
                    index,
                    &slot_handle,
                    lines,
                );
                if path.segments.len() == 2 {
                    self.populate_item_slot_from_item_def(
                        function,
                        depth,
                        value,
                        &slot_handle,
                        lines,
                    );
                } else if let Some(PathSegment::Field(field)) = path.segments.get(2) {
                    if field == "name" {
                        lines.push(format!(
                            "data modify storage {}:runtime {}.nbt.display.Name set from storage {}:runtime {}",
                            self.namespace,
                            slot_handle.storage_path(),
                            self.namespace,
                            value_slot.storage_path()
                        ));
                    } else {
                        let rendered = self.render_storage_path(
                            function,
                            depth,
                            slot_handle.storage_path().to_string(),
                            &Type::ItemSlot,
                            &path.segments[2..],
                            &path.segment_types[2..],
                            lines,
                        );
                        lines.push(self.storage_path_command(
                            format!(
                                "data modify storage {}:runtime {} set from storage {}:runtime {}",
                                self.namespace,
                                rendered.path,
                                self.namespace,
                                value_slot.storage_path()
                            ),
                            rendered.macro_storage,
                        ));
                    }
                }
                self.sync_item_slot_handle(function, depth, &slot_handle, lines);
                true
            }
            "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" => self
                .compile_player_mainhand_assign(
                    function,
                    depth,
                    base_slot,
                    first,
                    &path.segments[1..],
                    value,
                    lines,
                ),
            _ => false,
        }
    }

    fn try_compile_player_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Field(first)) = path.segments.first() else {
            return false;
        };
        match first.as_str() {
            "position" => {
                let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockRef);
                self.compose_entity_position_slot(base_slot, &pos_slot, lines);
                if path.segments.len() == 1 {
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        pos_slot.storage_path()
                    ));
                } else {
                    let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
                        &Type::BlockRef,
                        &path.segments[1..],
                    ));
                    lines.push(self.block_command(
                        &pos_slot,
                        format!(
                            "data modify storage {}:runtime {} set from block $(pos) {}",
                            self.namespace,
                            target.storage_path(),
                            path_text
                        ),
                        true,
                    ));
                }
                true
            }
            "nbt" => {
                if path.base.ref_kind != RefKind::Player {
                    return false;
                }
                let path_text = render_nbt_path_segments(&path.segments[1..]);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "data modify storage {}:runtime {} set from entity $(selector) {}",
                        self.namespace,
                        target.storage_path(),
                        path_text
                    ),
                    true,
                ));
                true
            }
            "state" => {
                if path.segments.len() == 1 {
                    return false;
                }
                let objective = state_objective(path.base.ref_kind, &path.segments[1..]);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute store result storage {}:runtime {} int 1 run scoreboard players get $(selector) {}",
                        self.namespace,
                        target.storage_path(),
                        objective
                    ),
                    true,
                ));
                true
            }
            "tags" => {
                if path.base.ref_kind != RefKind::Player {
                    return false;
                }
                let tag = render_path_segments(&path.segments[1..]);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value 0",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute as $(selector) if entity @s[tag={}] run data modify storage {}:runtime {} set value 1",
                        tag,
                        self.namespace,
                        target.storage_path()
                    ),
                    true,
                ));
                true
            }
            "team" => {
                lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.team",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                true
            }
            "inventory" | "hotbar" => {
                let Some((namespace, index)) = player_item_slot_index_from_path(path) else {
                    return false;
                };
                let slot_handle =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                self.load_player_item_slot(
                    function,
                    depth,
                    base_slot,
                    namespace,
                    index,
                    &slot_handle,
                    lines,
                );
                if path.segments.len() == 2 {
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        slot_handle.storage_path()
                    ));
                } else if let Some(PathSegment::Field(field)) = path.segments.get(2) {
                    if field == "name" {
                        lines.push(format!(
                            "data modify storage {}:runtime {} set value \"\"",
                            self.namespace,
                            target.storage_path()
                        ));
                        lines.push(format!(
                            "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {} set from storage {}:runtime {}.nbt.display.Name",
                            self.namespace,
                            slot_handle.storage_path(),
                            self.namespace,
                            target.storage_path(),
                            self.namespace,
                            slot_handle.storage_path()
                        ));
                    } else {
                        let rendered = self.render_storage_path(
                            function,
                            depth,
                            slot_handle.storage_path().to_string(),
                            &Type::ItemSlot,
                            &path.segments[2..],
                            &path.segment_types[2..],
                            lines,
                        );
                        self.compile_storage_read_from_path(rendered, &path.ty, target, lines);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn compile_item_slot_path_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        value_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if let Some(PathSegment::Field(field)) = path.segments.first() {
            if field == "name" {
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt.display.Name set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    value_slot.storage_path()
                ));
            } else {
                let rendered = self.render_storage_path(
                    function,
                    depth,
                    base_slot.storage_path().to_string(),
                    &Type::ItemSlot,
                    &path.segments,
                    &path.segment_types,
                    lines,
                );
                lines.push(self.storage_path_command(
                    format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        rendered.path,
                        self.namespace,
                        value_slot.storage_path()
                    ),
                    rendered.macro_storage,
                ));
            }
            self.sync_item_slot_handle(function, depth, base_slot, lines);
        }
    }

    fn compile_item_slot_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if let Some(PathSegment::Field(field)) = path.segments.first() {
            if field == "name" {
                lines.push(format!(
                    "data modify storage {}:runtime {} set value \"\"",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {} set from storage {}:runtime {}.nbt.display.Name",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                return;
            }
        }
        let rendered = self.render_storage_path(
            function,
            depth,
            base_slot.storage_path().to_string(),
            &Type::ItemSlot,
            &path.segments,
            &path.segment_types,
            lines,
        );
        self.compile_storage_read_from_path(rendered, &path.ty, target, lines);
    }

    fn load_player_item_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        namespace: &str,
        index: &crate::ast::Expr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            base_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            base_slot.storage_path()
        ));
        match &index.kind {
            crate::ast::ExprKind::Int(logical_index) => {
                let slot_index = player_slot_nbt_index(namespace, *logical_index);
                lines.push(format!(
                    "data modify storage {}:runtime {}.logical_slot set value {}",
                    self.namespace,
                    target.storage_path(),
                    logical_index
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.slot set value {}",
                    self.namespace,
                    target.storage_path(),
                    slot_index
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.command_slot set value {}",
                    self.namespace,
                    target.storage_path(),
                    quoted(&player_item_command_slot(namespace, *logical_index))
                ));
            }
            _ => {
                self.compile_expr_to_macro_value(
                    function,
                    depth,
                    index,
                    &Type::Int,
                    target.storage_path(),
                    "logical_slot",
                    lines,
                );
                if namespace == "inventory" {
                    let slot_index =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                    lines.push(format!(
                        "execute store result score {} mcfc run data get storage {}:runtime {}.logical_slot 1",
                        slot_index.numeric_name(),
                        self.namespace,
                        target.storage_path()
                    ));
                    lines.push(format!(
                        "scoreboard players add {} mcfc 9",
                        slot_index.numeric_name()
                    ));
                    lines.push(format!(
                        "execute store result storage {}:runtime {}.slot int 1 run scoreboard players get {} mcfc",
                        self.namespace,
                        target.storage_path(),
                        slot_index.numeric_name()
                    ));
                } else {
                    lines.push(format!(
                        "data modify storage {}:runtime {}.slot set from storage {}:runtime {}.logical_slot",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        target.storage_path()
                    ));
                }
                lines.push(self.inline_macro_command(
                    target.storage_path(),
                    format!(
                        "data modify storage {}:runtime {}.command_slot set value \"{}.$(logical_slot)\"",
                        self.namespace,
                        target.storage_path(),
                        namespace
                    ),
                ));
            }
        }
        let slot_path = "Inventory[{Slot:$(slot)b}]";
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set value \"\"",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run data modify storage {}:runtime {}.exists set value 1",
                slot_path,
                self.namespace,
                target.storage_path()
            ),
            true,
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run data modify storage {}:runtime {}.id set from entity $(selector) {}.id",
                slot_path,
                self.namespace,
                target.storage_path(),
                slot_path
            ),
            true,
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run execute store result storage {}:runtime {}.count int 1 run data get entity $(selector) {}.Count 1",
                slot_path,
                self.namespace,
                target.storage_path(),
                slot_path
            ),
            true,
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run data modify storage {}:runtime {}.nbt set from entity $(selector) {}",
                slot_path,
                self.namespace,
                target.storage_path(),
                slot_path
            ),
            true,
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.id",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.Count",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.Slot",
            self.namespace,
            target.storage_path()
        ));
    }

    fn populate_item_slot_from_item_def(
        &mut self,
        function: &IrFunction,
        depth: usize,
        value: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::ItemDef);
        self.compile_expr_into_slot(function, depth, value, &item_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 1",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
            self.namespace,
            target.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set from storage {}:runtime {}.count",
            self.namespace,
            target.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set from storage {}:runtime {}.nbt",
            self.namespace,
            target.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
    }

    fn clear_item_slot_handle(
        &mut self,
        function: &IrFunction,
        depth: usize,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set value \"\"",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        self.sync_item_slot_handle(function, depth, target, lines);
    }

    fn sync_item_slot_handle(
        &mut self,
        function: &IrFunction,
        depth: usize,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let exists_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
        lines.push(format!(
            "execute store result score {} mcfc run data get storage {}:runtime {}.exists 1",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path()
        ));
        lines.push(self.query_command(
            target,
            "item replace entity $(selector) $(command_slot) with air".to_string(),
            true,
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.item_name",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "execute if score {} mcfc matches 1 if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {}.item_name set from storage {}:runtime {}.nbt.display.Name",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path(),
            self.namespace,
            target.storage_path(),
            self.namespace,
            target.storage_path()
        ));
        let named_replace = self.query_command(
            target,
            "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_name='\"$(item_name)\"',minecraft:custom_data=$(nbt)] $(count)".to_string(),
            true,
        );
        let plain_replace = self.query_command(
            target,
            "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_data=$(nbt)] $(count)".to_string(),
            true,
        );
        lines.push(format!(
            "execute if score {} mcfc matches 1 if data storage {}:runtime {}.item_name run {}",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path(),
            named_replace
        ));
        lines.push(format!(
            "execute if score {} mcfc matches 1 unless data storage {}:runtime {}.item_name run {}",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path(),
            plain_replace
        ));
    }

    fn compile_player_mainhand_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        slot_name: &str,
        segments: &[PathSegment],
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Field(field)) = segments.first() else {
            return false;
        };
        match field.as_str() {
            "name" => {
                let name_temp = self.new_temp();
                let name_slot = local_slot(depth, &function.name, &name_temp, &Type::String);
                self.compile_expr_into_slot(function, depth, value, &name_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.item_name set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    name_slot.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "item modify entity $(selector) {} {{\"function\":\"minecraft:set_name\",\"name\":\"$(item_name)\",\"target\":\"custom_name\"}}",
                        equipment_slot_name(slot_name)
                    ),
                    true,
                ));
                true
            }
            "count" => {
                let count_temp = self.new_temp();
                let count_slot = local_slot(depth, &function.name, &count_temp, &Type::Int);
                self.compile_expr_into_slot(function, depth, value, &count_slot, lines);
                lines.push(format!(
                    "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    base_slot.storage_path(),
                    count_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "item modify entity $(selector) {} {{\"function\":\"minecraft:set_count\",\"count\":$(count)}}",
                        equipment_slot_name(slot_name)
                    ),
                    true,
                ));
                true
            }
            "item" => {
                if value.ty == Type::ItemDef {
                    let slot_handle =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
                        self.namespace,
                        slot_handle.storage_path(),
                        self.namespace,
                        base_slot.storage_path()
                    ));
                    lines.push(format!(
                        "data modify storage {}:runtime {}.command_slot set value {}",
                        self.namespace,
                        slot_handle.storage_path(),
                        quoted(equipment_slot_name(slot_name))
                    ));
                    self.populate_item_slot_from_item_def(
                        function,
                        depth,
                        value,
                        &slot_handle,
                        lines,
                    );
                    self.sync_item_slot_handle(function, depth, &slot_handle, lines);
                    return true;
                }
                let item_temp = self.new_temp();
                let item_slot = local_slot(depth, &function.name, &item_temp, &Type::String);
                self.compile_expr_into_slot(function, depth, value, &item_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.item_id set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    item_slot.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "item replace entity $(selector) {} with $(item_id)",
                        equipment_slot_name(slot_name)
                    ),
                    true,
                ));
                true
            }
            _ => false,
        }
    }

    fn compile_cast(
        &mut self,
        function: &IrFunction,
        depth: usize,
        kind: CastKind,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let temp_name = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp_name, &Type::Nbt);
        self.compile_expr_into_slot(function, depth, expr, &temp_slot, lines);
        match kind {
            CastKind::Int | CastKind::Bool => {
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {} 1",
                    target.numeric_name(),
                    self.namespace,
                    temp_slot.storage_path()
                ));
                if matches!(kind, CastKind::Bool) {
                    let raw = self.new_temp();
                    let raw_slot = local_slot(depth, &function.name, &raw, &Type::Int);
                    lines.push(format!(
                        "scoreboard players operation {} mcfc = {} mcfc",
                        raw_slot.numeric_name(),
                        target.numeric_name()
                    ));
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        target.numeric_name()
                    ));
                    lines.push(format!(
                        "execute unless score {} mcfc matches 0 run scoreboard players set {} mcfc 1",
                        raw_slot.numeric_name(),
                        target.numeric_name()
                    ));
                }
            }
            CastKind::String => lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                temp_slot.storage_path()
            )),
        }
    }

    fn compile_value_as_nbt(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        match expr.ty {
            Type::Nbt | Type::TextDef => {
                self.compile_expr_into_slot(function, depth, expr, target, lines)
            }
            Type::EntityDef => {
                let spec_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::EntityDef);
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!(
                        "data merge storage {}:runtime {} $(data)",
                        self.namespace,
                        target.storage_path()
                    ),
                ));
            }
            Type::BlockDef => {
                let spec_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::BlockDef);
                self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.nbt",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
            }
            Type::ItemDef => {
                let spec_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::ItemDef);
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let count_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {}.count 1",
                    count_slot.numeric_name(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.Count byte 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target.storage_path(),
                    count_slot.numeric_name()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!(
                        "data merge storage {}:runtime {} $(data)",
                        self.namespace,
                        target.storage_path()
                    ),
                ));
            }
            Type::Int | Type::Bool => {
                let temp = self.new_temp();
                let temp_slot = local_slot(depth, &function.name, &temp, &expr.ty);
                self.compile_expr_into_slot(function, depth, expr, &temp_slot, lines);
                lines.push(format!(
                    "execute store result storage {}:runtime {} int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target.storage_path(),
                    temp_slot.numeric_name()
                ));
            }
            Type::String
            | Type::Array(_)
            | Type::Dict(_)
            | Type::Struct(_)
            | Type::ItemSlot
            | Type::Bossbar => {
                self.compile_expr_into_slot(function, depth, expr, target, lines);
            }
            _ => {}
        }
    }

    fn write_query_slot(
        &self,
        target: &SlotRef,
        prefix: &str,
        selector: &str,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set value {}",
            self.namespace,
            target.storage_path(),
            quoted(prefix)
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set value {}",
            self.namespace,
            target.storage_path(),
            quoted(selector)
        ));
    }

    fn write_block_slot(&self, target: &SlotRef, prefix: &str, pos: &str, lines: &mut Vec<String>) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set value {}",
            self.namespace,
            target.storage_path(),
            quoted(prefix)
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.pos set value {}",
            self.namespace,
            target.storage_path(),
            quoted(pos)
        ));
    }

    fn compose_context_slots(
        &mut self,
        kind: ContextKind,
        anchor: &SlotRef,
        value: &SlotRef,
        target: &SlotRef,
        ty: &Type,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            anchor.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            anchor.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.__value_prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            value.storage_path()
        ));
        lines.push(self.inline_macro_command(
            target.storage_path(),
            format!(
                "data modify storage {}:runtime {}.prefix set value \"$(__anchor_prefix)execute {} $(__anchor_selector) run $(__value_prefix)\"",
                self.namespace,
                target.storage_path(),
                context_execute_keyword(kind)
            ),
        ));
        match ty {
            Type::EntitySet | Type::EntityRef | Type::PlayerRef => lines.push(format!(
                "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
                self.namespace,
                target.storage_path(),
                self.namespace,
                value.storage_path()
            )),
            Type::BlockRef => lines.push(format!(
                "data modify storage {}:runtime {}.pos set from storage {}:runtime {}.pos",
                self.namespace,
                target.storage_path(),
                self.namespace,
                value.storage_path()
            )),
            _ => {}
        }
    }

    fn compose_entity_position_slot(
        &mut self,
        entity: &SlotRef,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            entity.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            entity.storage_path()
        ));
        lines.push(self.inline_macro_command(
            target.storage_path(),
            format!(
                "data modify storage {}:runtime {}.prefix set value \"$(__anchor_prefix)execute at $(__anchor_selector) run \"",
                self.namespace,
                target.storage_path()
            ),
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.pos set value \"~ ~ ~\"",
            self.namespace,
            target.storage_path()
        ));
    }

    fn query_command(&mut self, slot: &SlotRef, command: String, wrap_macro: bool) -> String {
        let storage = slot.storage_path();
        let macro_line = format!("$(prefix){}", command);
        let relative = if wrap_macro { macro_line } else { command };
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(relative);
        format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage
        )
    }

    fn inline_macro_command(&mut self, storage_path: &str, command: String) -> String {
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(command);
        format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage_path
        )
    }

    fn block_command(&mut self, slot: &SlotRef, command: String, wrap_macro: bool) -> String {
        let storage = slot.storage_path();
        let macro_line = format!("$(prefix){}", command);
        let relative = if wrap_macro { macro_line } else { command };
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(relative);
        format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage
        )
    }

    fn ensure_inline_macro(&mut self, command: String) -> String {
        self.macro_counter += 1;
        let relative = format!("generated/internal_macro_{}", self.macro_counter);
        let path = format!("data/{}/function/{}.mcfunction", self.namespace, relative);
        self.files.insert(path, format!("${}\n", command));
        relative
    }

    fn compile_unary(
        &mut self,
        function: &IrFunction,
        depth: usize,
        op: UnaryOp,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let temp = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp, &expr.ty);
        self.compile_expr_into_slot(function, depth, expr, &temp_slot, lines);

        match op {
            UnaryOp::Not => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 1",
                    target.numeric_name()
                ));
                lines.push(format!(
                    "execute if score {} mcfc matches 1 run scoreboard players set {} mcfc 0",
                    temp_slot.numeric_name(),
                    target.numeric_name()
                ));
            }
        }
    }

    fn compile_binary(
        &mut self,
        function: &IrFunction,
        depth: usize,
        op: BinaryOp,
        left: &IrExpr,
        right: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        match op {
            BinaryOp::And => {
                let left_temp = self.new_temp();
                let left_slot = local_slot(depth, &function.name, &left_temp, &left.ty);
                self.compile_expr_into_slot(function, depth, left, &left_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                let (rhs_path, rhs_name) = self.new_block(function, depth, "logic_and_rhs");
                let mut rhs_lines = Vec::new();
                self.compile_expr_into_slot(function, depth, right, target, &mut rhs_lines);
                self.files.insert(rhs_path, rhs_lines.join("\n") + "\n");
                lines.push(format!(
                    "execute if score {} mcfc matches 1 run function {}:{}",
                    left_slot.numeric_name(),
                    self.namespace,
                    rhs_name
                ));
                return;
            }
            BinaryOp::Or => {
                let left_temp = self.new_temp();
                let left_slot = local_slot(depth, &function.name, &left_temp, &left.ty);
                self.compile_expr_into_slot(function, depth, left, &left_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 1",
                    target.numeric_name()
                ));
                let (rhs_path, rhs_name) = self.new_block(function, depth, "logic_or_rhs");
                let mut rhs_lines = Vec::new();
                self.compile_expr_into_slot(function, depth, right, target, &mut rhs_lines);
                self.files.insert(rhs_path, rhs_lines.join("\n") + "\n");
                lines.push(format!(
                    "execute if score {} mcfc matches 0 run function {}:{}",
                    left_slot.numeric_name(),
                    self.namespace,
                    rhs_name
                ));
                return;
            }
            _ => {}
        }

        let left_temp = self.new_temp();
        let right_temp = self.new_temp();
        let left_slot = local_slot(depth, &function.name, &left_temp, &left.ty);
        let right_slot = local_slot(depth, &function.name, &right_temp, &right.ty);
        self.compile_expr_into_slot(function, depth, left, &left_slot, lines);
        self.compile_expr_into_slot(function, depth, right, &right_slot, lines);

        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                lines.push(format!(
                    "scoreboard players operation {} mcfc = {} mcfc",
                    target.numeric_name(),
                    left_slot.numeric_name()
                ));
                let operator = match op {
                    BinaryOp::Add => "+=",
                    BinaryOp::Sub => "-=",
                    BinaryOp::Mul => "*=",
                    BinaryOp::Div => "/=",
                    _ => unreachable!(),
                };
                lines.push(format!(
                    "scoreboard players operation {} mcfc {} {} mcfc",
                    target.numeric_name(),
                    operator,
                    right_slot.numeric_name()
                ));
            }
            BinaryOp::Eq | BinaryOp::NotEq
                if matches!(left.ty, Type::String) && matches!(right.ty, Type::String) =>
            {
                self.compile_string_equality(op, &left_slot, &right_slot, target, lines);
            }
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Lte
            | BinaryOp::Gt
            | BinaryOp::Gte => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                let (keyword, operator) = match op {
                    BinaryOp::Eq => ("if", "="),
                    BinaryOp::NotEq => ("unless", "="),
                    BinaryOp::Lt => ("if", "<"),
                    BinaryOp::Lte => ("if", "<="),
                    BinaryOp::Gt => ("if", ">"),
                    BinaryOp::Gte => ("if", ">="),
                    _ => unreachable!(),
                };
                lines.push(format!(
                    "execute {} score {} mcfc {} {} mcfc run scoreboard players set {} mcfc 1",
                    keyword,
                    left_slot.numeric_name(),
                    operator,
                    right_slot.numeric_name(),
                    target.numeric_name()
                ));
            }
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    fn compile_string_equality(
        &mut self,
        op: BinaryOp,
        left_slot: &SlotRef,
        right_slot: &SlotRef,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let compare_storage = format!("frames.__cmp{}", self.new_temp());
        let compare_result = numeric_slot(0, "__cmp", &self.new_temp());
        lines.push(format!(
            "data modify storage {}:runtime {} set from storage {}:runtime {}",
            self.namespace,
            compare_storage,
            self.namespace,
            left_slot.storage_path()
        ));
        lines.push(format!(
            "execute store success score {} mcfc run data modify storage {}:runtime {} set from storage {}:runtime {}",
            compare_result,
            self.namespace,
            compare_storage,
            self.namespace,
            right_slot.storage_path()
        ));

        match op {
            BinaryOp::Eq => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(format!(
                    "execute if score {} mcfc matches 0 run scoreboard players set {} mcfc 1",
                    compare_result,
                    target.numeric_name()
                ));
            }
            BinaryOp::NotEq => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(format!(
                    "execute if score {} mcfc matches 1 run scoreboard players set {} mcfc 1",
                    compare_result,
                    target.numeric_name()
                ));
            }
            _ => unreachable!(),
        }
    }

    fn function_entry_name(&self, function: &str, depth: usize) -> String {
        format!("generated/{}__d{}__entry", sanitize(function), depth)
    }

    fn emit_macro_command(
        &mut self,
        function: &IrFunction,
        depth: usize,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        lines: &mut Vec<String>,
    ) {
        let template = expand_display_text_sugar(template);
        if placeholders.is_empty() {
            lines.push(template);
            return;
        }
        let rendered_template = rewrite_macro_template(&template, placeholders);

        self.macro_counter += 1;
        let macro_id = self.macro_counter;
        let relative = format!(
            "generated/{}__d{}__macro_{}",
            sanitize(&function.name),
            depth,
            macro_id
        );
        let path = format!("data/{}/function/{}.mcfunction", self.namespace, relative);
        self.files.insert(path, format!("${}\n", rendered_template));

        let storage_base = macro_storage_base(depth, &function.name, macro_id);
        for placeholder in placeholders {
            let source_temp = self.new_temp();
            let source_slot = local_slot(depth, &function.name, &source_temp, &placeholder.ty);
            self.compile_expr_into_slot(function, depth, &placeholder.expr, &source_slot, lines);
            let target_path = format!("{}.{}", storage_base, placeholder.key);
            match placeholder.ty {
                Type::Int | Type::Bool => lines.push(format!(
                    "execute store result storage {}:runtime {} int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_path,
                    source_slot.numeric_name()
                )),
                Type::String | Type::Nbt | Type::TextDef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::EntitySet | Type::EntityRef | Type::PlayerRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.selector",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::BlockRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.pos",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Array(_)
                | Type::Dict(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::ItemDef
                | Type::ItemSlot
                | Type::Bossbar => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Void => {}
            }
        }
        lines.push(format!(
            "function {}:{} with storage {}:runtime {}",
            self.namespace, relative, self.namespace, storage_base
        ));
    }

    fn compile_interpolated_string(
        &mut self,
        function: &IrFunction,
        depth: usize,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if placeholders.is_empty() {
            lines.push(format!(
                "data modify storage {}:runtime {} set value {}",
                self.namespace,
                target.storage_path(),
                quoted(template)
            ));
            return;
        }

        self.macro_counter += 1;
        let macro_id = self.macro_counter;
        let relative = format!(
            "generated/{}__d{}__string_{}",
            sanitize(&function.name),
            depth,
            macro_id
        );
        let path = format!("data/{}/function/{}.mcfunction", self.namespace, relative);
        let rendered_value = quoted(&rewrite_macro_template(template, placeholders));
        self.files.insert(
            path,
            format!(
                "$data modify storage {}:runtime {} set value {}\n",
                self.namespace,
                target.storage_path(),
                rendered_value
            ),
        );

        let storage_base = macro_storage_base(depth, &function.name, macro_id);
        self.write_macro_placeholder_values(function, depth, &storage_base, placeholders, lines);
        lines.push(format!(
            "function {}:{} with storage {}:runtime {}",
            self.namespace, relative, self.namespace, storage_base
        ));
    }

    fn write_macro_placeholder_values(
        &mut self,
        function: &IrFunction,
        depth: usize,
        storage_base: &str,
        placeholders: &[IrMacroPlaceholder],
        lines: &mut Vec<String>,
    ) {
        for placeholder in placeholders {
            let source_temp = self.new_temp();
            let source_slot = local_slot(depth, &function.name, &source_temp, &placeholder.ty);
            self.compile_expr_into_slot(function, depth, &placeholder.expr, &source_slot, lines);
            let target_path = format!("{}.{}", storage_base, placeholder.key);
            match placeholder.ty {
                Type::Int | Type::Bool => lines.push(format!(
                    "execute store result storage {}:runtime {} int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_path,
                    source_slot.numeric_name()
                )),
                Type::String | Type::Nbt | Type::TextDef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::EntitySet | Type::EntityRef | Type::PlayerRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.selector",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::BlockRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.pos",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Array(_)
                | Type::Dict(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::ItemDef
                | Type::ItemSlot
                | Type::Bossbar => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Void => {}
            }
        }
    }

    fn function_entry_path(&self, function: &str, depth: usize) -> String {
        format!(
            "data/{}/function/{}.mcfunction",
            self.namespace,
            self.function_entry_name(function, depth)
        )
    }

    fn new_block(&mut self, function: &IrFunction, depth: usize, label: &str) -> (String, String) {
        self.block_counter += 1;
        let relative = format!(
            "generated/{}__d{}__{}_{}",
            sanitize(&function.name),
            depth,
            label,
            self.block_counter
        );
        (
            format!("data/{}/function/{}.mcfunction", self.namespace, relative),
            relative,
        )
    }

    fn new_temp(&mut self) -> String {
        self.temp_counter += 1;
        format!("__tmp{}", self.temp_counter)
    }
}

fn continuation_after_stmts(stmts: &[IrStmt], tail: &[ContinuationItem]) -> Vec<ContinuationItem> {
    stmts
        .iter()
        .cloned()
        .map(ContinuationItem::Stmt)
        .chain(tail.iter().cloned())
        .collect()
}

#[derive(Debug, Clone)]
struct SlotRef {
    name: String,
}

impl SlotRef {
    fn numeric_name(&self) -> &str {
        &self.name
    }

    fn storage_path(&self) -> &str {
        &self.name
    }
}

fn local_slot(depth: usize, function: &str, name: &str, ty: &Type) -> SlotRef {
    match ty {
        Type::Int | Type::Bool => SlotRef {
            name: numeric_slot(depth, function, name),
        },
        Type::String
        | Type::Array(_)
        | Type::Dict(_)
        | Type::Struct(_)
        | Type::EntityDef
        | Type::BlockDef
        | Type::ItemDef
        | Type::TextDef
        | Type::ItemSlot
        | Type::Bossbar
        | Type::EntitySet
        | Type::EntityRef
        | Type::PlayerRef
        | Type::BlockRef
        | Type::Nbt => SlotRef {
            name: string_slot(depth, function, name),
        },
        Type::Void => SlotRef {
            name: "__void".to_string(),
        },
    }
}

fn return_slot(depth: usize, function: &str, ty: &Type) -> SlotRef {
    match ty {
        Type::Int | Type::Bool => SlotRef {
            name: numeric_return_slot(depth, function),
        },
        Type::String
        | Type::Array(_)
        | Type::Dict(_)
        | Type::Struct(_)
        | Type::EntityDef
        | Type::BlockDef
        | Type::ItemDef
        | Type::TextDef
        | Type::ItemSlot
        | Type::Bossbar
        | Type::EntitySet
        | Type::EntityRef
        | Type::PlayerRef
        | Type::BlockRef
        | Type::Nbt => SlotRef {
            name: string_return_slot(depth, function),
        },
        Type::Void => SlotRef {
            name: "__void".to_string(),
        },
    }
}

fn control_slot(depth: usize, function: &str) -> String {
    format!("$d{}_{}__ctrl", depth, sanitize(function))
}

fn numeric_slot(depth: usize, function: &str, name: &str) -> String {
    format!("$d{}_{}_{}", depth, sanitize(function), sanitize(name))
}

fn numeric_return_slot(depth: usize, function: &str) -> String {
    format!("$d{}_{}__ret", depth, sanitize(function))
}

fn string_slot(depth: usize, function: &str, name: &str) -> String {
    format!(
        "frames.d{}.{}.{}",
        depth,
        sanitize(function),
        sanitize(name)
    )
}

fn string_return_slot(depth: usize, function: &str) -> String {
    format!("frames.d{}.{}.__ret", depth, sanitize(function))
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn equipment_slot_name(slot_name: &str) -> &'static str {
    match slot_name {
        "mainhand" => "weapon.mainhand",
        "offhand" => "weapon.offhand",
        "head" => "armor.head",
        "chest" => "armor.chest",
        "legs" => "armor.legs",
        "feet" => "armor.feet",
        _ => "weapon.mainhand",
    }
}

fn quoted(value: &str) -> String {
    format!("{:?}", value)
}

fn expand_display_text_sugar(command: &str) -> String {
    expand_json_text_command_sugar(command)
        .or_else(|| expand_say_sugar(command))
        .unwrap_or_else(|| command.to_string())
}

fn expand_json_text_command_sugar(command: &str) -> Option<String> {
    if let Some(rest) = command.strip_prefix("tellraw ") {
        let (target, message) = split_first_word(rest)?;
        let json = quoted_message_to_selector_json(message.trim_start())?;
        return Some(format!("tellraw {} {}", target, json));
    }

    if let Some(rest) = command.strip_prefix("title ") {
        let (target, rest) = split_first_word(rest)?;
        let (mode, message) = split_first_word(rest.trim_start())?;
        if !matches!(mode, "title" | "subtitle" | "actionbar") {
            return None;
        }
        let json = quoted_message_to_selector_json(message.trim_start())?;
        return Some(format!("title {} {} {}", target, mode, json));
    }

    None
}

fn expand_say_sugar(command: &str) -> Option<String> {
    let message = command.strip_prefix("say ")?.trim_start();
    let text = parse_display_message(message)?;
    let json = selector_text_components(&text)?;
    Some(format!("tellraw @a {}", json))
}

fn quoted_message_to_selector_json(message: &str) -> Option<String> {
    let (text, trailing) = parse_quoted_message(message)?;
    trailing.trim().is_empty().then_some(())?;
    selector_text_components(&text)
}

fn display_text_components(text: &str) -> Option<String> {
    selector_text_components_with_self_replacement(text, Some("$(selector)"))
}

fn split_first_word(value: &str) -> Option<(&str, &str)> {
    let split_at = value
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index))?;
    Some((&value[..split_at], &value[split_at..]))
}

fn parse_quoted_message(value: &str) -> Option<(String, &str)> {
    let mut chars = value.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }

    let mut text = String::new();
    let mut escaped = false;
    for (index, ch) in chars {
        if escaped {
            text.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some((text, &value[index + ch.len_utf8()..])),
            _ => text.push(ch),
        }
    }
    None
}

fn parse_display_message(value: &str) -> Option<String> {
    if let Some((text, trailing)) = parse_quoted_message(value) {
        if trailing.trim().is_empty() {
            return Some(text);
        }
    }
    Some(value.to_string())
}

fn selector_text_components(text: &str) -> Option<String> {
    selector_text_components_with_self_replacement(text, None)
}

fn selector_text_components_with_self_replacement(
    text: &str,
    self_replacement: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut changed = false;
    let mut index = 0usize;

    while index < text.len() {
        let rest = &text[index..];
        if let Some(selector_len) = selector_token_len(rest) {
            if !plain.is_empty() {
                parts.push(quoted(&plain));
                plain.clear();
            }
            let selector = &rest[..selector_len];
            let rendered_selector = if selector == "@s" {
                self_replacement.unwrap_or(selector)
            } else {
                selector
            };
            parts.push(format!("{{\"selector\":{}}}", quoted(rendered_selector)));
            index += selector_len;
            changed = true;
        } else {
            let ch = rest.chars().next().expect("non-empty string");
            plain.push(ch);
            index += ch.len_utf8();
        }
    }

    if !plain.is_empty() {
        parts.push(quoted(&plain));
    }

    changed.then(|| format!("[{}]", parts.join(",")))
}

fn selector_token_len(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'@' || !matches!(bytes[1], b'p' | b'a' | b'r' | b's' | b'e')
    {
        return None;
    }

    if bytes.get(2) != Some(&b'[') {
        return Some(2);
    }

    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices().skip(2) {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch == '[' {
            depth += 1;
        } else if ch == ']' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index + ch.len_utf8());
            }
        }
    }

    Some(2)
}

fn context_execute_keyword(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::As => "as",
        ContextKind::At => "at",
    }
}

fn render_path_segments(segments: &[PathSegment]) -> String {
    let mut rendered = String::new();
    for segment in segments {
        match segment {
            PathSegment::Field(name) => push_path_name(&mut rendered, name),
            PathSegment::Index(index) => {
                let value = match &index.kind {
                    crate::ast::ExprKind::Int(value) => *value,
                    _ => 0,
                };
                rendered.push_str(&format!("[{}]", value));
            }
        }
    }
    rendered
}

fn render_nbt_path_segments(segments: &[PathSegment]) -> String {
    let mut rendered = String::new();
    for segment in segments {
        match segment {
            PathSegment::Field(name) => push_path_name(&mut rendered, name),
            PathSegment::Index(index) => match &index.kind {
                crate::ast::ExprKind::Int(value) => rendered.push_str(&format!("[{}]", value)),
                crate::ast::ExprKind::String(value) => push_quoted_path_name(&mut rendered, value),
                _ => rendered.push_str("[0]"),
            },
        }
    }
    rendered
}

fn push_path_name(rendered: &mut String, name: &str) {
    if !rendered.is_empty() {
        rendered.push('.');
    }
    rendered.push_str(name);
}

fn push_quoted_path_name(rendered: &mut String, name: &str) {
    if !rendered.is_empty() {
        rendered.push('.');
    }
    rendered.push_str(&quoted(name));
}

fn push_quoted_macro_path_name(rendered: &mut String, placeholder: &str) {
    if !rendered.is_empty() {
        rendered.push('.');
    }
    rendered.push('"');
    rendered.push_str(&format!("$({})", placeholder));
    rendered.push('"');
}

fn normalize_runtime_nbt_segments<'a>(
    base_ty: &Type,
    segments: &'a [PathSegment],
) -> &'a [PathSegment] {
    if matches!(base_ty, Type::EntityRef | Type::PlayerRef | Type::BlockRef)
        && segments.len() > 1
        && matches!(segments.first(), Some(PathSegment::Field(field)) if field == "nbt")
    {
        &segments[1..]
    } else {
        segments
    }
}

fn infer_dynamic_nbt_index_type(function: &IrFunction, expr: &crate::ast::Expr) -> Option<Type> {
    match &expr.kind {
        crate::ast::ExprKind::Int(_) => Some(Type::Int),
        crate::ast::ExprKind::String(_) => Some(Type::String),
        crate::ast::ExprKind::Variable(name) => function.locals.get(name).cloned(),
        crate::ast::ExprKind::Unary { expr, .. } => infer_dynamic_nbt_index_type(function, expr),
        crate::ast::ExprKind::Binary { .. }
        | crate::ast::ExprKind::Call { .. }
        | crate::ast::ExprKind::MethodCall { .. }
        | crate::ast::ExprKind::Bool(_)
        | crate::ast::ExprKind::Path(_)
        | crate::ast::ExprKind::ArrayLiteral(_)
        | crate::ast::ExprKind::DictLiteral(_)
        | crate::ast::ExprKind::StructLiteral { .. } => None,
    }
}

fn rewrite_macro_template(template: &str, placeholders: &[IrMacroPlaceholder]) -> String {
    let bytes = template.as_bytes();
    let mut index = 0usize;
    let mut out = String::new();
    let mut placeholder_index = 0usize;
    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b'$' && bytes[index + 1] == b'(' {
            let start = index + 2;
            index = start;
            let mut paren_depth = 1usize;
            let mut in_string = false;
            let mut string_delim = b'"';
            while index < bytes.len() {
                let ch = bytes[index];
                if in_string {
                    if ch == b'\\' {
                        index += 2;
                        continue;
                    }
                    if ch == string_delim {
                        in_string = false;
                    }
                    index += 1;
                    continue;
                }
                match ch {
                    b'"' | b'\'' => {
                        in_string = true;
                        string_delim = ch;
                    }
                    b'(' => paren_depth += 1,
                    b')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            if let Some(placeholder) = placeholders.get(placeholder_index) {
                out.push_str(&format!("$({})", placeholder.key));
                placeholder_index += 1;
            }
        } else {
            out.push(bytes[index] as char);
        }
        index += 1;
    }
    out
}

fn player_state_objective(segments: &[PathSegment]) -> String {
    format!("mcfs_{}", sanitize(&render_path_segments(segments)))
}

fn entity_state_objective(segments: &[PathSegment]) -> String {
    format!("mcfe_{}", sanitize(&render_path_segments(segments)))
}

fn state_objective(ref_kind: RefKind, segments: &[PathSegment]) -> String {
    if ref_kind == RefKind::Player {
        player_state_objective(segments)
    } else {
        entity_state_objective(segments)
    }
}

fn player_item_slot_index_from_path(path: &IrPathExpr) -> Option<(&str, &crate::ast::Expr)> {
    let PathSegment::Field(namespace) = path.segments.first()? else {
        return None;
    };
    if !matches!(namespace.as_str(), "inventory" | "hotbar") {
        return None;
    }
    let PathSegment::Index(index) = path.segments.get(1)? else {
        return None;
    };
    Some((namespace.as_str(), index))
}

fn player_slot_nbt_index(namespace: &str, logical_index: i64) -> i64 {
    match namespace {
        "inventory" => logical_index + 9,
        _ => logical_index,
    }
}

fn player_item_command_slot(namespace: &str, logical_index: i64) -> String {
    match namespace {
        "inventory" => format!("inventory.{}", logical_index),
        _ => format!("hotbar.{}", logical_index),
    }
}

fn collect_block_builder_state_fields(
    program: &IrProgram,
) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
    let mut fields = BTreeMap::<String, BTreeMap<String, BTreeMap<String, ()>>>::new();
    for function in &program.functions {
        collect_block_builder_state_fields_from_stmts(&function.name, &function.body, &mut fields);
    }
    fields
        .into_iter()
        .map(|(function, vars)| {
            (
                function,
                vars.into_iter()
                    .map(|(name, fields)| (name, fields.into_keys().collect()))
                    .collect(),
            )
        })
        .collect()
}

fn collect_block_builder_state_fields_from_stmts(
    function_name: &str,
    stmts: &[IrStmt],
    fields: &mut BTreeMap<String, BTreeMap<String, BTreeMap<String, ()>>>,
) {
    for stmt in stmts {
        match stmt {
            IrStmt::Assign {
                target: IrAssignTarget::Path(path),
                ..
            } => collect_block_builder_state_fields_from_path(function_name, path, fields),
            IrStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_block_builder_state_fields_from_stmts(function_name, then_body, fields);
                collect_block_builder_state_fields_from_stmts(function_name, else_body, fields);
            }
            IrStmt::While { body, .. }
            | IrStmt::Context { body, .. }
            | IrStmt::For { body, .. } => {
                collect_block_builder_state_fields_from_stmts(function_name, body, fields);
            }
            IrStmt::Async { function, .. } => {
                collect_block_builder_state_fields_from_stmts(
                    &function.name,
                    &function.body,
                    fields,
                );
            }
            _ => {}
        }
    }
}

fn collect_block_builder_state_fields_from_path(
    function_name: &str,
    path: &IrPathExpr,
    fields: &mut BTreeMap<String, BTreeMap<String, BTreeMap<String, ()>>>,
) {
    if path.base.ty != Type::BlockDef {
        return;
    }
    let IrExprKind::Variable(name) = &path.base.kind else {
        return;
    };
    let [PathSegment::Field(root), PathSegment::Field(field), ..] = path.segments.as_slice() else {
        return;
    };
    if root != "states" {
        return;
    }
    fields
        .entry(function_name.to_string())
        .or_default()
        .entry(name.clone())
        .or_default()
        .insert(field.clone(), ());
}

fn collect_state_objectives(program: &IrProgram) -> Vec<ManagedObjective> {
    let mut names = BTreeMap::<String, Option<String>>::new();
    for state in &program.player_states {
        let segments = state
            .path
            .iter()
            .map(|segment| PathSegment::Field(segment.clone()))
            .collect::<Vec<_>>();
        names.insert(
            player_state_objective(&segments),
            Some(state.display_name.clone()),
        );
    }
    for function in &program.functions {
        collect_objectives_from_stmts(&function.body, &mut names);
    }
    names
        .into_iter()
        .map(|(objective, display_name)| ManagedObjective {
            objective,
            display_name,
        })
        .collect()
}

fn collect_objectives_from_stmts(stmts: &[IrStmt], names: &mut BTreeMap<String, Option<String>>) {
    for stmt in stmts {
        match stmt {
            IrStmt::Assign {
                target: IrAssignTarget::Path(path),
                ..
            } => collect_objectives_from_path(path, names),
            IrStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_objectives_from_expr(condition, names);
                collect_objectives_from_stmts(then_body, names);
                collect_objectives_from_stmts(else_body, names);
            }
            IrStmt::While { condition, body } => {
                collect_objectives_from_expr(condition, names);
                collect_objectives_from_stmts(body, names);
            }
            IrStmt::For { kind, body, .. } => {
                match kind {
                    IrForKind::Range { start, end, .. } => {
                        collect_objectives_from_expr(start, names);
                        collect_objectives_from_expr(end, names);
                    }
                    IrForKind::Each { iterable } => collect_objectives_from_expr(iterable, names),
                }
                collect_objectives_from_stmts(body, names);
            }
            IrStmt::Context { anchor, body, .. } => {
                collect_objectives_from_expr(anchor, names);
                collect_objectives_from_stmts(body, names);
            }
            IrStmt::Async { function, .. } => {
                collect_objectives_from_stmts(&function.body, names);
            }
            IrStmt::Let { value, .. }
            | IrStmt::Return(Some(value))
            | IrStmt::Sleep {
                duration: value, ..
            }
            | IrStmt::Expr(value) => collect_objectives_from_expr(value, names),
            IrStmt::MacroCommand { .. }
            | IrStmt::RawCommand(_)
            | IrStmt::Break
            | IrStmt::Continue
            | IrStmt::Return(None)
            | IrStmt::Assign { .. } => {}
        }
    }
}

fn collect_objectives_from_expr(expr: &IrExpr, names: &mut BTreeMap<String, Option<String>>) {
    match &expr.kind {
        IrExprKind::Path(path) => collect_objectives_from_path(path, names),
        IrExprKind::Unary { expr, .. }
        | IrExprKind::Single(expr)
        | IrExprKind::Exists(expr)
        | IrExprKind::HasData(expr)
        | IrExprKind::Cast { expr, .. } => collect_objectives_from_expr(expr, names),
        IrExprKind::Binary { left, right, .. } => {
            collect_objectives_from_expr(left, names);
            collect_objectives_from_expr(right, names);
        }
        IrExprKind::Call { args, .. } => {
            for arg in args {
                collect_objectives_from_expr(arg, names);
            }
        }
        IrExprKind::ArrayLiteral(values) => {
            for value in values {
                collect_objectives_from_expr(value, names);
            }
        }
        IrExprKind::DictLiteral(entries) => {
            for (_, value) in entries {
                collect_objectives_from_expr(value, names);
            }
        }
        IrExprKind::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_objectives_from_expr(value, names);
            }
        }
        IrExprKind::MethodCall { receiver, args, .. } => {
            collect_objectives_from_expr(receiver, names);
            for arg in args {
                collect_objectives_from_expr(arg, names);
            }
        }
        IrExprKind::InterpolatedString { placeholders, .. } => {
            for placeholder in placeholders {
                collect_objectives_from_expr(&placeholder.expr, names);
            }
        }
        IrExprKind::At { anchor, value } | IrExprKind::As { anchor, value } => {
            collect_objectives_from_expr(anchor, names);
            collect_objectives_from_expr(value, names);
        }
        IrExprKind::Int(_)
        | IrExprKind::Bool(_)
        | IrExprKind::String(_)
        | IrExprKind::Variable(_)
        | IrExprKind::Selector(_)
        | IrExprKind::Block(_) => {}
    }
}

fn collect_objectives_from_path(path: &IrPathExpr, names: &mut BTreeMap<String, Option<String>>) {
    if path.segments.len() > 1
        && matches!(path.segments.first(), Some(PathSegment::Field(name)) if name == "state")
        && matches!(path.base.ty, Type::EntityRef | Type::PlayerRef)
    {
        names
            .entry(state_objective(path.base.ref_kind, &path.segments[1..]))
            .or_insert(None);
    }
    collect_objectives_from_expr(&path.base, names);
}

fn macro_storage_base(depth: usize, function: &str, macro_id: usize) -> String {
    format!(
        "frames.d{}.{}.__macro{}",
        depth,
        sanitize(function),
        macro_id
    )
}

fn render_tag_file(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| format!("    \"{}\"", value))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n  \"values\": [\n{}\n  ]\n}}\n", body)
}

fn has_special_tick(program: &IrProgram) -> bool {
    program.functions.iter().any(|function| {
        !function.generated
            && function.name == "tick"
            && function.params.is_empty()
            && function.return_type == Type::Void
    })
}
