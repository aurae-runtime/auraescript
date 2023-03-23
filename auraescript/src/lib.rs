/* -------------------------------------------------------------------------- *\
 *        Apache 2.0 License Copyright © 2022-2023 The Aurae Authors          *
 *                                                                            *
 *                +--------------------------------------------+              *
 *                |   █████╗ ██╗   ██╗██████╗  █████╗ ███████╗ |              *
 *                |  ██╔══██╗██║   ██║██╔══██╗██╔══██╗██╔════╝ |              *
 *                |  ███████║██║   ██║██████╔╝███████║█████╗   |              *
 *                |  ██╔══██║██║   ██║██╔══██╗██╔══██║██╔══╝   |              *
 *                |  ██║  ██║╚██████╔╝██║  ██║██║  ██║███████╗ |              *
 *                |  ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝ |              *
 *                +--------------------------------------------+              *
 *                                                                            *
 *                         Distributed Systems Runtime                        *
 *                                                                            *
 * -------------------------------------------------------------------------- *
 *                                                                            *
 *   Licensed under the Apache License, Version 2.0 (the "License");          *
 *   you may not use this file except in compliance with the License.         *
 *   You may obtain a copy of the License at                                  *
 *                                                                            *
 *       http://www.apache.org/licenses/LICENSE-2.0                           *
 *                                                                            *
 *   Unless required by applicable law or agreed to in writing, software      *
 *   distributed under the License is distributed on an "AS IS" BASIS,        *
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. *
 *   See the License for the specific language governing permissions and      *
 *   limitations under the License.                                           *
 *                                                                            *
\* -------------------------------------------------------------------------- */

// Lint groups: https://doc.rust-lang.org/rustc/lints/groups.html
#![warn(future_incompatible, nonstandard_style, unused)]
#![warn(
    improper_ctypes,
    non_shorthand_field_patterns,
    no_mangle_generic_items,
    unconditional_recursion,
    unused_comparisons,
    while_true
)]
#![warn(missing_debug_implementations,
    // TODO: missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]
#![warn(clippy::unwrap_used)]
// TODO: need to figure out how to get tonic to allow this without allowing for whole crate
#![allow(unused_qualifications)]

use anyhow::{anyhow, bail, Error};
use deno_ast::{MediaType, ParseParams, SourceTextInfo};
use deno_core::futures::FutureExt;
use deno_core::url::Url;
use deno_core::{
    resolve_import, Extension, JsRuntime, ModuleLoader, ModuleSource,
    ModuleSourceFuture, ModuleSpecifier, ModuleType, OpDecl, RuntimeOptions,
};
use deno_runtime::worker::MainWorker;
use std::pin::Pin;
use std::rc::Rc;

mod builtin;
mod cells;
mod cri;
mod discovery;
mod health;
mod observe;

pub fn init(main_module: Url) -> MainWorker {
    let extension = Extension::builder("main-worker").ops(stdlib()).build();

    let create_web_worker_cb = Arc::new(|_| {
        todo!("Web workers are not supported yet");
    });
    let web_worker_event_cb = Arc::new(|_| {
        todo!("Web workers are not supported yet");
    });

    MainWorker::bootstrap_from_options(
        main_module,
        PermissionsContainer::allow_all(),
        WorkerOptions {
            extensions: vec![extension],
            module_loader: Some(Rc::new(TypescriptModuleLoader)),
            bootstrap: BootstrapOptions {
                args: vec![],
                cpu_count: 1,
                debug_flag: false,
                enable_testing_features: false,
                locale: deno_core::v8::icu::get_language_tag(),
                location: None,
                no_color: false,
                is_tty: false,
                runtime_version: "x".to_string(),
                ts_version: "x".to_string(),
                unstable: true,
                user_agent: "xactdb_runtime".to_string(),
                inspect: false,
            },
            extensions_with_js: vec![],
            startup_snapshot: None,
            unsafely_ignore_certificate_errors: None,
            root_cert_store: None,
            seed: None,
            source_map_getter: None,
            format_js_error_fn: None,
            web_worker_preload_module_cb: web_worker_event_cb.clone(),
            web_worker_pre_execute_module_cb: web_worker_event_cb,
            create_web_worker_cb,
            maybe_inspector_server: None,
            should_break_on_first_statement: false,
            should_wait_for_inspector_session: false,
            npm_resolver: None,
            get_error_class_fn: Some(&get_error_class_name),
            cache_storage_dir: None,
            origin_storage_dir: None,
            blob_store: BlobStore::default(),
            broadcast_channel: InMemoryBroadcastChannel::default(),
            shared_array_buffer_store: None,
            compiled_wasm_module_store: None,
            stdio: Default::default(),
        },
    )
}

/// Standard Library Autogeneration Code
///
/// To add an auto generated package to AuraeScript it MUST
/// be defined in this function.
///
/// Add a similar line to the function for each newly implemented
/// service.
///
/// ops.extend(my_package::op_decls());
///
fn stdlib() -> Vec<OpDecl> {
    let mut ops = vec![];
    ops.extend(cells::op_decls());
    ops.extend(cri::op_decls());
    ops.extend(discovery::op_decls());
    ops.extend(health::op_decls());
    ops.extend(observe::op_decls());
    ops
}

// From: https://github.com/denoland/deno/blob/main/core/examples/ts_module_loader.rs
struct TypescriptModuleLoader;

impl ModuleLoader for TypescriptModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _is_main: bool,
    ) -> Result<ModuleSpecifier, Error> {
        Ok(resolve_import(specifier, referrer)?)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<ModuleSpecifier>,
        _is_dyn_import: bool,
    ) -> Pin<Box<ModuleSourceFuture>> {
        let module_specifier = module_specifier.clone();
        async move {
            let path = module_specifier
                .to_file_path()
                .map_err(|_| anyhow!("Only file: URLs are supported."))?;

            let media_type = MediaType::from(&path);
            let (module_type, should_transpile) = match MediaType::from(&path) {
                MediaType::JavaScript | MediaType::Mjs | MediaType::Cjs => {
                    (ModuleType::JavaScript, false)
                }
                MediaType::Jsx => (ModuleType::JavaScript, true),
                MediaType::TypeScript
                | MediaType::Mts
                | MediaType::Cts
                | MediaType::Dts
                | MediaType::Dmts
                | MediaType::Dcts
                | MediaType::Tsx => (ModuleType::JavaScript, true),
                MediaType::Json => (ModuleType::Json, false),
                _ => bail!("Unknown extension {:?}", path.extension()),
            };

            let code = std::fs::read_to_string(&path)?;
            let code = if should_transpile {
                let parsed = deno_ast::parse_module(ParseParams {
                    specifier: module_specifier.to_string(),
                    text_info: SourceTextInfo::from_string(code),
                    media_type,
                    capture_tokens: false,
                    scope_analysis: false,
                    maybe_syntax: None,
                })?;
                parsed.transpile(&Default::default())?.text
            } else {
                code
            };
            let module = ModuleSource {
                code: code.into_bytes().into_boxed_slice(),
                module_type,
                module_url_specified: module_specifier.to_string(),
                module_url_found: module_specifier.to_string(),
            };
            Ok(module)
        }
        .boxed_local()
    }
}
