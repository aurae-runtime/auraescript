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

//! Systems daemon built for higher order simple, safe, secure multi-tenant
//! distributed systems.
//!
//! Whether run as pid 1 (init), or a Container, or a Pod it serves standard library
//! functionality over an mTLS backed gRPC server.
//!
//! The Aurae Daemon (auraed) is the main server implementation of the Aurae
//! Standard Library.
//!
//! See [`The Aurae Standard Library`] for API reference.
//!
//! [`The Aurae Standard Library`]: https://aurae.io/stdlib
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
#![warn(
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_results
)]
#![warn(clippy::unwrap_used)]

use crate::cri::oci::AuraeOCIBuilder;
use crate::{
    cells::CellService, cri::runtime_service::RuntimeService,
    discovery::DiscoveryService, ebpf::loader::BpfLoader,
    init::Context as AuraeContext, init::SocketStream,
    logging::log_channel::LogChannel, observe::ObserveService,
    spawn::spawn_auraed_oci_to,
};
use anyhow::Context;
use once_cell::sync::OnceCell;
use proto::{
    cells::cell_service_server::CellServiceServer,
    cri::runtime_service_server::RuntimeServiceServer,
    discovery::discovery_service_server::DiscoveryServiceServer,
    observe::observe_service_server::ObserveServiceServer,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tonic::transport::server::Connected;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{error, info, trace, warn};

mod cells;
mod cri;
mod discovery;
mod ebpf;
mod graceful_shutdown;
mod init;
mod logging;
mod observe;
mod spawn;
mod vms;

static AURAED_RUNTIME: OnceCell<AuraedRuntime> = OnceCell::new();

/// Each instance of Aurae holds internal state in memory. Below are the
/// settings which can be configured for a given Aurae daemon instance.
///
/// Note: These fields represent file paths and not the actual authentication
/// material. Each new instance of a subsystem will read these from the local
/// filesystem at runtime in order to authenticate.
#[derive(Debug)]
pub struct AuraedRuntime {
    /// Certificate Authority for an organization or mesh of Aurae instances.
    pub ca_crt: PathBuf,
    /// The signed server X509 certificate for this unique instance.
    pub server_crt: PathBuf,
    /// The secret key for this unique instance.
    pub server_key: PathBuf,
    /// Configurable runtime directory. Defaults to /var/run/aurae.
    pub runtime_dir: PathBuf,
    /// Configurable library directory. Defaults to /var/lib/aurae.
    pub library_dir: PathBuf,
    // /// Provides logging channels to expose auraed logging via grpc
    //pub log_collector: Arc<LogChannel>,
}

impl AuraedRuntime {
    pub(crate) fn bundles_dir(&self) -> PathBuf {
        self.runtime_dir.join("bundles")
    }

    pub(crate) fn pods_dir(&self) -> PathBuf {
        self.runtime_dir.join("pods")
    }

    pub(crate) fn default_socket_address(&self) -> PathBuf {
        self.runtime_dir.join("aurae.sock")
    }
}

impl Default for AuraedRuntime {
    fn default() -> Self {
        // In order to prevent there use from other areas, do not make values into constants.
        AuraedRuntime {
            ca_crt: PathBuf::from("/etc/aurae/pki/ca.crt"),
            server_crt: PathBuf::from("/etc/aurae/pki/_signed.server.crt"),
            server_key: PathBuf::from("/etc/aurae/pki/server.key"),
            runtime_dir: PathBuf::from("/var/run/aurae"),
            library_dir: PathBuf::from("/var/lib/aurae"),
        }
    }
}

/// Starts the runtime loop for the daemon.
pub async fn run(
    runtime: AuraedRuntime,
    socket: Option<String>,
    verbose: bool,
    nested: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    async fn inner<T, IO, IE>(
        runtime: &AuraedRuntime,
        context: AuraeContext,
        socket_stream: T,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        T: tokio_stream::Stream<Item = Result<IO, IE>> + Send + 'static,
        IO: AsyncRead + AsyncWrite + Connected + Unpin + Send + 'static,
        IE: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        trace!("{:#?}", runtime);

        let server_crt =
            tokio::fs::read(&runtime.server_crt).await.with_context(|| {
                format!(
                    "Aurae requires a signed TLS certificate to run as a server, but failed to
                    load: '{}'. Please see https://aurae.io/certs/ for information on best
                    practices to quickly generate one.",
                    runtime.server_crt.display()
                )
            })?;
        let server_key = tokio::fs::read(&runtime.server_key).await?;
        let server_identity = Identity::from_pem(server_crt, server_key);
        info!("Register Server SSL Identity");

        let ca_crt = tokio::fs::read(&runtime.ca_crt).await?;
        let ca_crt_pem = Certificate::from_pem(ca_crt.clone());

        let tls = ServerTlsConfig::new()
            .identity(server_identity)
            .client_ca_root(ca_crt_pem);

        info!("Validating SSL Identity and Root Certificate Authority (CA)");
        //let _log_collector = self.log_collector.clone();

        let runtime_dir = Path::new(&runtime.runtime_dir);
        // Create runtime directory
        tokio::fs::create_dir_all(runtime_dir).await.with_context(|| {
            format!(
                "Failed to create runtime directory: {}",
                runtime.runtime_dir.display()
            )
        })?;

        // Install eBPF probes in the host Aurae daemon
        let (_bpf_scope, signals) = if context == AuraeContext::Cell
            || context == AuraeContext::Container
        {
            (None, None)
        } else {
            // TODO: Add flags/options to "opt-out" of the various BPF probes
            info!("Loading eBPF probes");
            let mut bpf_loader = BpfLoader::new();
            let listener = bpf_loader
                .read_and_load_tracepoint_signal_signal_generate()
                .ok();

            if listener.is_none() {
                warn!("Missing eBPF probe. Skipping signal reporting.");
            }

            // Need to move bpf_loader out to prevent it from being dropped
            (Some(bpf_loader), listener)
        };

        // Build gRPC Services
        let (mut health_reporter, health_service) =
            tonic_health::server::health_reporter();

        let cell_service = CellService::new();
        let cell_service_server = CellServiceServer::new(cell_service.clone());
        health_reporter.set_serving::<CellServiceServer<CellService>>().await;

        let discovery_service = DiscoveryService::new();
        let discovery_service_server =
            DiscoveryServiceServer::new(discovery_service);
        health_reporter
            .set_serving::<DiscoveryServiceServer<DiscoveryService>>()
            .await;

        let observe_service = ObserveService::new(
            Arc::new(LogChannel::new(String::from("TODO"))),
            signals,
        );
        let observe_service_server = ObserveServiceServer::new(observe_service);
        health_reporter
            .set_serving::<ObserveServiceServer<ObserveService>>()
            .await;

        // let pod_service = PodService::new(self.runtime_dir.clone());
        // let pod_service_server = PodServiceServer::new(pod_service.clone());
        // health_reporter.set_serving::<PodServiceServer<PodService>>().await;
        let runtime_service = RuntimeService::new();
        let runtime_service_server =
            RuntimeServiceServer::new(runtime_service.clone());
        health_reporter
            .set_serving::<RuntimeServiceServer<RuntimeService>>()
            .await;

        // let vm_service = VmService::new();
        // let vm_service_server = VmServiceServer::new(vm_service.clone());
        // health_reporter.set_serving::<VmServiceServer<VmService>>().await;

        let graceful_shutdown = graceful_shutdown::GracefulShutdown::new(
            health_reporter,
            cell_service,
        );
        let graceful_shutdown_signal = graceful_shutdown.subscribe();

        // Run the server concurrently
        // TODO: pass a known-good path to CellService to store any runtime data.
        let server_handle = tokio::spawn(async move {
            Server::builder()
                .tls_config(tls)?
                .add_service(health_service)
                .add_service(cell_service_server)
                .add_service(discovery_service_server)
                .add_service(observe_service_server)
                // .add_service(pod_service_server)
                .add_service(runtime_service_server)
                // .add_service(vm_service_server)
                .serve_with_incoming_shutdown(socket_stream, async {
                    let mut graceful_shutdown_signal = graceful_shutdown_signal;
                    let _ = graceful_shutdown_signal.changed().await;
                    info!("gRPC server received shutdown signal...");
                })
                .await?;

            info!("gRPC server exited successfully");

            Ok::<_, tonic::transport::Error>(())
        });

        // Event loop
        let graceful_shutdown_handle =
            tokio::spawn(async { graceful_shutdown.wait().await });

        let (server_result, _) =
            tokio::try_join!(server_handle, graceful_shutdown_handle)?;

        if let Err(e) = server_result {
            error!("gRPC server exited with error: {e}");
        }

        Ok(())
    }

    let runtime = AURAED_RUNTIME.get_or_init(|| runtime);

    let (context, stream) = init::init(verbose, nested, socket).await;
    match stream {
        SocketStream::Tcp(stream) => inner(runtime, context, stream).await,
        SocketStream::Unix(stream) => inner(runtime, context, stream).await,
    }
}

/// Write the container OCI spec to the filesystem in preparation for spawning Auraed using a container runtime.
pub fn prep_oci_spec_for_spawn(output: &str) {
    spawn_auraed_oci_to(
        PathBuf::from(output),
        AuraeOCIBuilder::new().build().expect("building default oci spec"),
    )
    .expect("spawning");
}
