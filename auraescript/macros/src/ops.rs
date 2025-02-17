/* -------------------------------------------------------------------------- *\
 *                |   █████╗ ██╗   ██╗██████╗  █████╗ ███████╗ |              *
 *                |  ██╔══██╗██║   ██║██╔══██╗██╔══██╗██╔════╝ |              *
 *                |  ███████║██║   ██║██████╔╝███████║█████╗   |              *
 *                |  ██╔══██║██║   ██║██╔══██╗██╔══██║██╔══╝   |              *
 *                |  ██║  ██║╚██████╔╝██║  ██║██║  ██║███████╗ |              *
 *                |  ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝ |              *
 *                +--------------------------------------------+              *
 *                                                                            *
 *                         Distributed Systems Runtime                        *
 * -------------------------------------------------------------------------- *
 * Copyright 2022 - 2024, the aurae contributors                              *
 * SPDX-License-Identifier: Apache-2.0                                        *
\* -------------------------------------------------------------------------- */
use heck::{ToLowerCamelCase, ToSnakeCase};
use proc_macro::TokenStream;
use proc_macro2::Ident;
use protobuf::descriptor::ServiceDescriptorProto;
use protobuf_parse::ParsedAndTypechecked;
use quote::quote;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parse_macro_input, Lit, Path, Token};

struct OpsGeneratorInput {
    file_path: Lit,
    module: Path,
    service_names: Punctuated<Ident, Token![,]>,
}

impl Parse for OpsGeneratorInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let file_path: Lit = input.parse()?;
        let _: Token![,] = input.parse()?;
        let module = input.parse()?;
        let _: Token![,] = input.parse()?;
        let service_names = input.parse_terminated(Ident::parse)?;

        Ok(Self { file_path, module, service_names })
    }
}

pub(crate) fn ops_generator(input: TokenStream) -> TokenStream {
    let OpsGeneratorInput { file_path, module, service_names } =
        parse_macro_input!(input as OpsGeneratorInput);

    let file_path_span = file_path.span();

    let (file_path, proto) = proto_reader::parse(&file_path);

    typescript_generator(&file_path, &module, &proto, &service_names);

    let output: Vec<(
        Vec<proc_macro2::TokenStream>,
        Vec<proc_macro2::TokenStream>,
    )> = proto
        .file_descriptors
        .iter()
        .flat_map(|f| &f.service)
        .filter(
            |s| matches!(s.name(), n if service_names.iter().any(|sn| sn == n)),
        )
        .map(|s| {
            let service_name_in_snake_case = Ident::new(&s.name().to_snake_case(), service_names.span());
            let client_ident =
                Ident::new(&format!("{}Client", s.name()), file_path_span);

            // TODO: support streaming
            let methods = s.method.iter().filter(|m| !m.client_streaming() && !m.server_streaming());

            let op_idents = methods.clone()
                .map(|m| {
                    Ident::new(
                        &op_name(&module, s.name(), m.name()),
                        file_path_span,
                    )
                });

            // generate a fn for each deno op
            let op_functions: Vec<proc_macro2::TokenStream> = methods
                .zip(op_idents.clone())
                .map(|(m, op_ident)| {
                    let input_type = proto_reader::helpers::to_unqualified_type(m.input_type());
                    let input_type = Ident::new(input_type, file_path_span);
                    let output_type = proto_reader::helpers::to_unqualified_type(m.output_type());
                    let output_type = Ident::new(output_type, file_path_span);
                    let name = Ident::new(&m.name().to_snake_case(), file_path_span);

                    // Magic OpState from deno (https://github.com/denoland/deno/blob/b6ac54815c1bcfa44a45b3f2c1c982829482477f/ops/lib.rs#L295)
                    quote! {
                        #[::deno_core::op2(async)]
                        #[serde]
                        pub(crate) async fn #op_ident(
                            op_state: Rc<RefCell<OpState>>, // Auto filled by deno macro, call from typescript ignoring this parameter
                            #[smi] client_rid: Option<::deno_core::ResourceId>,
                            #[serde] req: ::proto::#module::#input_type,
                        ) -> std::result::Result<
                            ::proto::#module::#output_type,
                            ::deno_error::JsErrorBox
                        > {
                            let client = match client_rid {
                                None => ::deno_core::RcRef::new(::client::Client::default().await
                                    .map_err(|e| ::deno_error::JsErrorBox::generic(
                                        format!("Failed to create default client: {:?}",e.to_string()))
                                    )?),
                                Some(client_rid) => {
                                    let as_client = {
                                        let op_state = &op_state.borrow();
                                        let rt = &op_state.resource_table; // get `ResourceTable` from JsRuntime `OpState`
                                        rt.get::<crate::builtin::auraescript_client::AuraeScriptClient>(client_rid) // get `Client` from its rid
                                    .map_err(|e| ::deno_error::JsErrorBox::generic(
                                                format!("Failed to get client: {:?}",e.to_string()))) // fix client error
                                            ?.clone()
                                    };
                                    ::deno_core::RcRef::map(as_client, |v| &v.0)
                                }
                            };
                            let res = ::client::#module::#service_name_in_snake_case::#client_ident::#name(
                                &(*client),
                                req
                            ).await.map_err(|e| ::deno_error::JsErrorBox::generic(
                                    format!("Failed call method {:?},{:?}: {:?}",
                                        stringify!(#service_name_in_snake_case),
                                        stringify!(#name),
                                        e.to_string())))?;

                            Ok(res.into_inner())
                        }
                    }
                })
                .collect();

            // generate a OpDecl for each function for conveniently adding to the deno runtime
            let op_decls: Vec<proc_macro2::TokenStream> = op_idents.map(|op_ident| {
                quote! {
                    #op_ident()
                }
            }).collect();

            (op_functions, op_decls)
        })
        .collect();

    let op_functions = output.iter().map(|x| &x.0);
    let op_decls = output.iter().map(|x| &x.1);

    let expanded = quote! {
        use ::std::{rc::Rc, cell::RefCell};
        use ::deno_core::{self, op2, OpState};

        #(#(#op_functions)*)*

        pub(crate) fn op_decls() -> Vec<::deno_core::OpDecl> {
            vec![#(#(#op_decls,)*)*]
        }
    };

    expanded.into()
}

/// Generates typescript implementations for multiple services by relying on
/// [typescript_service_generator] for each. Then outputs a concatenated file of the protoc
/// generated typescript with the service implementations to the gen directory.
fn typescript_generator(
    file_path: &std::path::Path,
    module: &Path,
    proto: &ParsedAndTypechecked,
    service_names: &Punctuated<Ident, Token![,]>,
) {
    // for each service, generate the service implementation and join them to a single string
    let services = proto
        .file_descriptors
        .iter()
        .flat_map(|f| &f.service)
        .filter(
            |s| matches!(s.name(), n if service_names.iter().any(|sn| sn == n)),
        )
        .map(|s| typescript_service_generator(module, s))
        .collect::<Vec<String>>()
        .join("\n\n");

    let gen_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(out_dir) => {
            let mut out_dir = PathBuf::from(out_dir);
            out_dir.push("gen");
            out_dir
        }
        _ => panic!("Environment variable 'CARGO_MANIFEST_DIR' was not set. Unable to locate crate root"),
    };

    let file_path = file_path
        .to_string_lossy()
        .splitn(2, "/api/")
        .last()
        .expect("path relative to gen directory")
        .replace(".proto", ".ts");

    let ts_path = gen_dir.join(file_path);

    // Open the generated ts file
    let mut ts =
        OpenOptions::new().read(true).open(ts_path.clone()).unwrap_or_else(
            |_| panic!("protoc output should generate {ts_path:?}"),
        );

    // read its contents
    let mut ts_contents = {
        let mut contents = String::new();
        match ts.read_to_string(&mut contents) {
            Ok(0) => panic!("{ts_path:?} is empty"),
            Err(e) => panic!("Failed to read {ts_path:?}: {e}"),
            _ => {}
        };
        contents
    };

    // concatenate the generated service implementations
    ts_contents.push_str(&services);

    // output a new file to the gen directory (overwrite if necessary)
    let ts_path = {
        let mut out_dir = gen_dir;
        out_dir.push(format!("{}.ts", path_to_snake_case(module)));
        out_dir
    };

    let mut ts = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(ts_path.clone())
        .unwrap_or_else(|_| {
            panic!("Failed to create or overwrite {ts_path:?}")
        });

    write!(ts, "{ts_contents}")
        .unwrap_or_else(|_| panic!("Could not write to {ts_path:?}"));
}

/// Returns typescript that implements a service by calling Deno ops.
fn typescript_service_generator(
    module: &Path,
    service: &ServiceDescriptorProto,
) -> String {
    let service_name = service.name();
    let mut ts_funcs: String = format!(
        r#"
export class {service_name}Client implements {service_name} {{
    client: number | undefined

    constructor(client?: number) {{
        this.client = client;
    }}
"#
    );

    service.method.iter().for_each(|m| {
        let method_name = m.name();
        let op_name = op_name(module, service.name(), method_name);
        let fn_name = method_name.to_lower_camel_case();
        let input_type =
            proto_reader::helpers::to_unqualified_type(m.input_type());
        let output_type =
            proto_reader::helpers::to_unqualified_type(m.output_type());

        ts_funcs.push_str(&format!(
            r#"
{fn_name}(request: {input_type}): Promise<{output_type}> {{
    // @ts-ignore
    return Deno.core.ops.{op_name}(this.client, request);
}}
        "#
        ));
    });

    ts_funcs.push('}');
    ts_funcs
}

/// Converts a path to snake case (e.g., grpc::health -> "grpc_health")
fn path_to_snake_case(path: &Path) -> String {
    path.segments
        .iter()
        .map(|x| x.ident.to_string().to_snake_case())
        .collect::<Vec<String>>()
        .join("_")
}

/// Example `ae__runtime__cell_service__allocate`
fn op_name(module: &Path, service_name: &str, method_name: &str) -> String {
    format!(
        "ae__{}__{}__{}",
        path_to_snake_case(module),
        service_name.to_snake_case(),
        method_name.to_snake_case()
    )
}
