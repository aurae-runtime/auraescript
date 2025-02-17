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
#![allow(non_snake_case)]

use anyhow::Result;
use client::{AuraeConfig, Client};
use deno_core::{self, op2, OpState, Resource, ResourceId};
use deno_error::JsErrorBox;
use std::{cell::RefCell, rc::Rc};

// `AuraeConfig` `try_default`
#[op2(fast)]
#[smi]
pub(crate) fn as__aurae_config__try_default(
    op_state: &mut OpState,
) -> Result<ResourceId, JsErrorBox> {
    let config = AuraeConfig::try_default().map_err(|err| {
        JsErrorBox::new("Failed to get default AuraeConfig", err.to_string())
    })?;
    let rid = op_state.resource_table.add(AuraeScriptConfig(config));
    Ok(rid)
}

// `AuraeConfig` `from_options`
#[op2(fast)]
#[smi]
pub(crate) fn as__aurae_config__from_options(
    op_state: &mut OpState,
    #[string] ca_crt: String,
    #[string] client_crt: String,
    #[string] client_key: String,
    #[string] socket: String,
) -> ResourceId {
    let config =
        AuraeConfig::from_options(ca_crt, client_crt, client_key, socket);
    op_state.resource_table.add(AuraeScriptConfig(config))
}

// `AuraeConfig` `parse_from_file`
#[op2(fast)]
#[smi]
pub(crate) fn as__aurae_config__parse_from_file(
    op_state: &mut OpState,
    #[string] path: String,
) -> Result<ResourceId, JsErrorBox> {
    let config = AuraeConfig::parse_from_toml_file(path).map_err(|err| {
        JsErrorBox::new(
            "Failed to parse AuraeConfig from toml file",
            err.to_string(),
        )
    })?;
    let rid = op_state.resource_table.add(AuraeScriptConfig(config));
    Ok(rid)
}

// Re export AuraeConfig in auraescript to be able to impl Resource on it
pub(crate) struct AuraeScriptConfig(pub AuraeConfig);

impl Resource for AuraeScriptConfig {} // Blank impl

// Create a `Client` with given `AuraeConfig`
#[op2(async)]
#[smi]
pub(crate) async fn as__client_new(
    op_state: Rc<RefCell<OpState>>,
    #[smi] config: ResourceId,
) -> Result<ResourceId, JsErrorBox> {
    let config = {
        let op_state = &op_state.borrow();
        let rt = &op_state.resource_table; // get `ResourceTable` from JsRuntime `OpState`
        rt.get::<AuraeScriptConfig>(config)
            .map_err(|err| {
                JsErrorBox::new(
                    "Failed to get AuraeConfig from its resource id",
                    err.to_string(),
                )
            })?
            .0
            .clone() // get `Config` from its rid
    };
    let client = Client::new(config).await.map_err(|err| {
        JsErrorBox::new("Failed to crate a client", err.to_string())
    })?;
    let mut op_state = op_state.borrow_mut();
    let rid = op_state.resource_table.add(AuraeScriptClient(client));
    Ok(rid)
}

pub(crate) fn op_decls() -> Vec<::deno_core::OpDecl> {
    vec![
        as__aurae_config__try_default(),
        as__aurae_config__from_options(),
        as__aurae_config__parse_from_file(),
        as__client_new(),
    ]
}

// Re export client in auraescript to be able
// to impl Resource on it
pub(crate) struct AuraeScriptClient(pub Client);

impl Resource for AuraeScriptClient {} // Blank impl
