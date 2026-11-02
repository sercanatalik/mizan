//! Script adapters: TypeScript run in a sandboxed QuickJS engine.
//!
//! Types are stripped at load time with oxc (no type checking; CI type-checks
//! bundled scripts). Each run gets a fresh runtime with a memory limit, a time
//! limit, no I/O, no clock and no randomness. The only host API is the
//! `mizan` object (decimal and date parsing, logging).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Exception, Function, Module, Object, Runtime, Value as JsValue};
use serde_json::{json, Value};

use crate::input::TableOptions;
use crate::manifest::{FileKind, Manifest};
use crate::mapping::{parse_date, parse_number};
use crate::run::{AdapterError, RunOutput};
use crate::{Adapter, FileInput};

pub const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
pub const TIME_LIMIT: Duration = Duration::from_secs(10);

/// Remove TypeScript syntax, returning plain JavaScript.
pub fn strip_types(source: &str, name: &str) -> Result<String, String> {
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_transformer::{TransformOptions, Transformer};

    let allocator = Allocator::default();
    let source_type = SourceType::ts().with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(e) = parsed.diagnostics.first() {
        return Err(format!("{name}: {}", describe(source, e)));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let options = TransformOptions::default();
    let ret = Transformer::new(&allocator, std::path::Path::new(name), &options)
        .build_with_scoping(scoping, &mut program);
    if let Some(e) = ret.diagnostics.first() {
        return Err(format!("{name}: {}", describe(source, e)));
    }
    Ok(Codegen::new().build(&program).code)
}

/// "message (line N)" from an oxc diagnostic.
fn describe(source: &str, e: &oxc_diagnostics::OxcDiagnostic) -> String {
    let offset = e.labels.as_ref().first().map(|l| l.offset() as usize);
    match offset {
        Some(o) => format!(
            "{} (line {})",
            e.message,
            source[..o.min(source.len())].matches('\n').count() + 1
        ),
        None => e.message.to_string(),
    }
}

/// Runs before the adapter: remove the clock and randomness.
const PRELUDE: &str = r#"
const RealDate = Date;
const noClock = () => { throw new Error("the clock is not available to adapters"); };
const SafeDate = new Proxy(RealDate, {
  construct(target, args) { if (args.length === 0) noClock(); return new target(...args); },
  apply() { noClock(); },
  get(target, prop) { if (prop === "now") return noClock; return Reflect.get(target, prop); },
});
globalThis.Date = SafeDate;
Math.random = () => { throw new Error("randomness is not available to adapters"); };
"#;

pub struct ScriptAdapter {
    manifest: Manifest,
    js: String,
    input: TableOptions,
}

pub fn build(manifest: Manifest, script: &str) -> Result<Arc<dyn Adapter>, String> {
    Ok(Arc::new(ScriptAdapter::new(
        manifest,
        script,
        TableOptions::default(),
    )?))
}

impl ScriptAdapter {
    pub fn new(manifest: Manifest, script: &str, input: TableOptions) -> Result<Self, String> {
        let js = strip_types(script, manifest.script.as_deref().unwrap_or("script.ts"))?;
        Ok(ScriptAdapter {
            manifest,
            js,
            input,
        })
    }

    /// The input object scripts receive.
    fn input_json(&self, file: &FileInput) -> Result<Value, String> {
        let mut v = json!({ "fileName": file.name, "kind": file.kind.as_str() });
        match file.kind {
            FileKind::Csv | FileKind::Xlsx => {
                let t = file.table(&self.input)?;
                // Header first, then data rows; every row keeps its source row number.
                let first = t
                    .rows
                    .first()
                    .map(|r| r.0.saturating_sub(1))
                    .unwrap_or(self.input.skip_rows + 1);
                let mut rows = vec![json!({ "row": first, "cells": t.header })];
                rows.extend(
                    t.rows
                        .iter()
                        .map(|(n, cells)| json!({ "row": n, "cells": cells })),
                );
                v["rows"] = Value::Array(rows);
            }
            FileKind::Pdf => v["pages"] = json!(file.pdf_pages()?),
            _ => v["text"] = json!(file.text(self.input.encoding.as_deref())?),
        }
        Ok(v)
    }

    /// Call `function_name(input)` in a fresh sandbox; returns the JSON result and log lines.
    fn call(
        &self,
        function_name: &str,
        input: &Value,
    ) -> Result<(Value, Vec<String>), AdapterError> {
        let id = &self.manifest.id;
        let fail = |m: String| AdapterError::new(id, m);
        let rt = Runtime::new().map_err(|e| fail(e.to_string()))?;
        rt.set_memory_limit(MEMORY_LIMIT);
        let deadline = Instant::now() + TIME_LIMIT;
        let timed_out = Rc::new(RefCell::new(false));
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag2 = flag.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            let over = Instant::now() > deadline;
            if over {
                flag2.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            over
        })));
        let ctx = Context::full(&rt).map_err(|e| fail(e.to_string()))?;
        let logs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let result = ctx.with(|ctx| -> Result<String, AdapterError> {
            let js_err = |ctx: &Ctx, e: rquickjs::Error| -> AdapterError {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    *timed_out.borrow_mut() = true;
                    return fail("time limit exceeded".into());
                }
                exception_to_error(id, ctx, e)
            };
            install_api(&ctx, logs.clone()).map_err(|e| js_err(&ctx, e))?;
            ctx.eval::<(), _>(PRELUDE).map_err(|e| js_err(&ctx, e))?;
            let (module, promise) = Module::declare(ctx.clone(), "adapter", self.js.as_str())
                .and_then(|m| m.eval())
                .map_err(|e| js_err(&ctx, e))?;
            promise.finish::<()>().map_err(|e| js_err(&ctx, e))?;
            let f: Function = module.get(function_name).map_err(|_| {
                fail(format!(
                    "the script must export a `{function_name}` function"
                ))
            })?;
            let arg = ctx
                .json_parse(input.to_string())
                .map_err(|e| js_err(&ctx, e))?;
            let ret: JsValue = f.call((arg,)).map_err(|e| js_err(&ctx, e))?;
            let text = ctx.json_stringify(ret).map_err(|e| js_err(&ctx, e))?;
            Ok(text
                .map(|s| s.to_string().unwrap_or_default())
                .unwrap_or_else(|| "null".into()))
        })?;
        let value = serde_json::from_str(&result)
            .map_err(|e| fail(format!("the script returned invalid JSON: {e}")))?;
        let lines = logs.borrow().clone();
        Ok((value, lines))
    }
}

/// The `mizan` object: pure helpers only.
fn install_api<'js>(ctx: &Ctx<'js>, logs: Rc<RefCell<Vec<String>>>) -> rquickjs::Result<()> {
    let mizan = Object::new(ctx.clone())?;
    mizan.set(
        "decimal",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, text: String, opts: rquickjs::function::Opt<Object<'js>>| {
                let (dec, thou) = match opts.0 {
                    Some(o) => (
                        o.get::<_, Option<String>>("decimal")?.unwrap_or(".".into()),
                        o.get::<_, Option<String>>("thousands")?.unwrap_or_default(),
                    ),
                    None => (".".into(), String::new()),
                };
                parse_number(&text, &dec, &thou).ok_or_else(|| {
                    Exception::throw_message(&ctx, &format!("{text:?} is not a number"))
                })
            },
        )?,
    )?;
    mizan.set(
        "date",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, text: String, format: String| {
                parse_date(&text, &format).ok_or_else(|| {
                    Exception::throw_message(
                        &ctx,
                        &format!("{text:?} does not match date format {format}"),
                    )
                })
            },
        )?,
    )?;
    mizan.set(
        "log",
        Function::new(ctx.clone(), move |msg: String| {
            logs.borrow_mut().push(msg);
        })?,
    )?;
    ctx.globals().set("mizan", mizan)?;
    Ok(())
}

/// Turn a JavaScript exception into an [`AdapterError`], keeping `locator` if the script set one.
fn exception_to_error(id: &str, ctx: &Ctx, e: rquickjs::Error) -> AdapterError {
    if !matches!(e, rquickjs::Error::Exception) {
        let msg = e.to_string();
        let msg = if msg.to_lowercase().contains("memory") {
            "memory limit exceeded".to_string()
        } else {
            msg
        };
        return AdapterError::new(id, msg);
    }
    let thrown = ctx.catch();
    if let Some(obj) = thrown.as_object() {
        let message = obj
            .get::<_, Option<String>>("message")
            .ok()
            .flatten()
            .unwrap_or_else(|| "exception".into());
        let message = if message.to_lowercase().contains("out of memory") {
            "memory limit exceeded".into()
        } else {
            message
        };
        let locator = obj.get::<_, Option<String>>("locator").ok().flatten();
        let mut err = AdapterError::new(id, message);
        err.locator = locator;
        return err;
    }
    let text = thrown
        .as_string()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| "exception".into());
    AdapterError::new(id, text)
}

impl Adapter for ScriptAdapter {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn detect(&self, file: &FileInput) -> u8 {
        if !self.manifest.files.contains(&file.kind) {
            return 0;
        }
        let Ok(input) = self.input_json(file) else {
            return 0;
        };
        match self.call("detect", &input) {
            Ok((v, _)) => v.as_f64().map(|n| n.clamp(0.0, 100.0) as u8).unwrap_or(0),
            Err(_) => 0,
        }
    }

    fn run(&self, file: &FileInput) -> Result<RunOutput, AdapterError> {
        let id = &self.manifest.id;
        let input = self
            .input_json(file)
            .map_err(|e| AdapterError::new(id, e))?;
        let (v, logs) = self.call("run", &input)?;
        let list = |key: &str| -> Result<Vec<Value>, AdapterError> {
            match v.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(a)) => Ok(a.clone()),
                Some(_) => Err(AdapterError::new(id, format!("`{key}` must be an array"))),
            }
        };
        if !v.is_object() {
            return Err(AdapterError::new(
                id,
                "`run` must return an object with `records`",
            ));
        }
        let mut diagnostics = list("diagnostics")?;
        diagnostics.extend(
            logs.into_iter()
                .map(|m| json!({ "severity": "info", "message": m, "locator": "script log" })),
        );
        Ok(RunOutput {
            records: list("records")?,
            assertions: list("assertions")?,
            diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::AdapterKind;

    fn manifest() -> Manifest {
        Manifest {
            id: "test-script".into(),
            version: "0.1.0".into(),
            name: "Test".into(),
            kind: AdapterKind::Script,
            files: vec![FileKind::Csv, FileKind::Text],
            native: None,
            script: Some("script.ts".into()),
            description: None,
        }
    }

    fn adapter(src: &str) -> ScriptAdapter {
        ScriptAdapter::new(manifest(), src, TableOptions::default()).unwrap()
    }

    fn csv() -> FileInput {
        FileInput::new("a.csv", b"Date;Amount\n2026-10-01;1.000,50\n".to_vec())
    }

    #[test]
    fn strips_types() {
        let js = strip_types(
            "interface Row { a: string }\nexport function f(x: Row): number { return (x as any).a.length as number; }\ntype T = string;",
            "s.ts",
        )
        .unwrap();
        assert!(js.contains("export function f(x)"), "{js}");
        assert!(!js.contains("interface") && !js.contains(": Row"), "{js}");
        let err = strip_types("const a = 1;\nconst b = ;\n", "s.ts").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }

    #[test]
    fn working_script_with_helpers() {
        let a = adapter(
            r#"
interface Row { row: number; cells: string[] }
export function detect(input: { rows: Row[] }): number { return input.rows[0].cells[0] === "Date" ? 80 : 0; }
export function run(input: { rows: Row[] }) {
  const records = input.rows.slice(1).map((r: Row) => ({
    locator: `row ${r.row}`, account: { hint: "A1" }, kind: "deposit",
    date: mizan.date(r.cells[0], "%Y-%m-%d"),
    net_cash: { amount: mizan.decimal(r.cells[1], { decimal: ",", thousands: "." }), currency: "EUR" },
  }));
  mizan.log(`read ${records.length} rows`);
  return { records };
}"#,
        );
        assert_eq!(a.detect(&csv()), 80);
        let out = a.run(&csv()).unwrap();
        assert_eq!(out.records[0]["net_cash"]["amount"], "1000.50");
        assert_eq!(out.records[0]["locator"], "row 2");
        assert_eq!(out.diagnostics[0]["message"], "read 1 rows");
    }

    #[test]
    fn time_limit() {
        let a = adapter("export function run() { while (true) {} }");
        let started = Instant::now();
        let err = a.run(&csv()).unwrap_err();
        assert_eq!(err.message, "time limit exceeded");
        assert!(started.elapsed() < TIME_LIMIT + Duration::from_secs(5));
    }

    #[test]
    fn memory_limit() {
        let a = adapter("export function run() { const a = []; while (true) { a.push(new Array(100000).fill('x')); } }");
        let err = a.run(&csv()).unwrap_err();
        assert!(err.message.contains("memory"), "{err}");
    }

    #[test]
    fn no_network_no_clock_no_randomness() {
        let err = adapter("export function run() { return fetch('https://example.com'); }")
            .run(&csv())
            .unwrap_err();
        assert!(err.message.contains("fetch"), "{err}");
        let err = adapter("export function run() { return { records: [], at: Date.now() }; }")
            .run(&csv())
            .unwrap_err();
        assert!(err.message.contains("clock"), "{err}");
        let err = adapter("export function run() { new Date(); return {}; }")
            .run(&csv())
            .unwrap_err();
        assert!(err.message.contains("clock"), "{err}");
        let ok = adapter("export function run() { return { records: [], d: new Date(2026, 0, 1).getFullYear() }; }").run(&csv());
        assert!(ok.is_ok(), "explicit dates are allowed: {ok:?}");
        let err = adapter("export function run() { return { n: Math.random() }; }")
            .run(&csv())
            .unwrap_err();
        assert!(err.message.contains("randomness"), "{err}");
    }

    #[test]
    fn exceptions_keep_their_locator() {
        let err = adapter(r#"export function run() { throw Object.assign(new Error("unexpected header"), { locator: "row 12" }); }"#)
            .run(&csv())
            .unwrap_err();
        assert_eq!(
            (err.message.as_str(), err.locator.as_deref()),
            ("unexpected header", Some("row 12"))
        );
        assert_eq!(
            err.to_string(),
            "adapter test-script failed at row 12: unexpected header"
        );
        let err = adapter("export function run() { return mizan.decimal('abc'); }")
            .run(&csv())
            .unwrap_err();
        assert!(err.message.contains("not a number"), "{err}");
        let err = adapter("export const x = 1;").run(&csv()).unwrap_err();
        assert!(err.message.contains("export a `run` function"), "{err}");
    }
}
