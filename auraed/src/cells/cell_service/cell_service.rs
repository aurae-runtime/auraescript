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

use super::{
    cells::{CellName, Cells, CellsCache},
    error::CellsServiceError,
    executables::Executables,
    validation::{
        ValidatedCellServiceAllocateRequest, ValidatedCellServiceFreeRequest,
        ValidatedCellServiceStartRequest, ValidatedCellServiceStopRequest,
    },
    Result,
};
use crate::{cells::cell_service::cells::CellsError, observe::ObserveService};
use ::validation::ValidatedType;
use backoff::backoff::Backoff;
use client::{cells::cell_service::CellServiceClient, Client, ClientError};
use proto::{
    cells::{
        cell_service_server, Cell, CellGraphNode, CellServiceAllocateRequest,
        CellServiceAllocateResponse, CellServiceFreeRequest,
        CellServiceFreeResponse, CellServiceListRequest,
        CellServiceListResponse, CellServiceStartRequest,
        CellServiceStartResponse, CellServiceStopRequest,
        CellServiceStopResponse, CpuController, CpusetController,
        MemoryController,
    },
    observe::LogChannelType,
};
use std::os::unix::fs::MetadataExt;
use std::time::Duration;
use std::{process::ExitStatus, sync::Arc};
use tokio::sync::Mutex;
use tonic::{Code, Request, Response, Status};
use tracing::{info, trace, warn};

/**
 * Macro to perform an operation within a cell.
 * It retries the operation with an exponential backoff strategy in case of connection errors.
 */
macro_rules! do_in_cell {
    ($self:ident, $cell_name:ident, $function:ident, $request:ident) => {{
        let mut cells = $self.cells.lock().await;

        // Retrieve the client socket for the specified cell
        let client_socket = cells
            .get(&$cell_name, |cell| cell.client_socket())
            .map_err(CellsServiceError::CellsError)?;

        // Initialize the exponential backoff strategy for retrying the operation
        let mut retry_strategy = backoff::ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_millis(50)) // 1st retry in 50ms
            .with_multiplier(10.0) // 10x the delay each attempt
            .with_randomization_factor(0.5) // with a randomness of +/-50%
            .with_max_interval(Duration::from_secs(3)) // but never delay more than 3s
            .with_max_elapsed_time(Some(Duration::from_secs(20))) // or 20s total
            .build();

        // Attempt to create a new client with retries in case of connection errors
        let client = loop {
            match Client::new_no_tls(client_socket.clone()).await {
                Ok(client) => break Ok(client),
                e @ Err(ClientError::ConnectionError(_)) => {
                    trace!("aurae client failed to connect: {e:?}");
                    if let Some(delay) = retry_strategy.next_backoff() {
                        trace!("retrying in {delay:?}");
                        tokio::time::sleep(delay).await
                    } else {
                        break e
                    }
                }
                e => break e
            }
        }.map_err(CellsServiceError::from)?;

        // Attempt the operation with the backoff strategy
        backoff::future::retry(
            retry_strategy,
            || async {
                match client.$function($request.clone()).await {
                    Ok(res) => Ok(res),
                    Err(e) if e.code() == Code::Unknown && e.message() == "transport error" => {
                        Err(e)?;
                        unreachable!();
                    }
                    Err(e) => Err(backoff::Error::Permanent(e))
                }
            },
        )
        .await
    }};
}

/// CellService struct manages the lifecycle of cells and executables.
#[derive(Debug, Clone)]
pub struct CellService {
    cells: Arc<Mutex<Cells>>,
    executables: Arc<Mutex<Executables>>,
    observe_service: ObserveService,
}

impl CellService {
    /// Creates a new instance of CellService.
    ///
    /// # Arguments
    /// * `observe_service` - An instance of ObserveService to manage log channels.
    pub fn new(observe_service: ObserveService) -> Self {
        CellService {
            cells: Default::default(),
            executables: Default::default(),
            observe_service,
        }
    }

    /// Allocates a new cell based on the provided request.
    ///
    /// # Arguments
    /// * `request` - A validated request to allocate a cell.
    ///
    /// # Returns
    /// A result containing the CellServiceAllocateResponse or an error.
    /// Frees an existing cell based on the provided request.
    #[tracing::instrument(skip(self))]
    async fn allocate(
        &self,
        request: ValidatedCellServiceAllocateRequest,
    ) -> Result<CellServiceAllocateResponse> {
        // Initialize the cell
        let ValidatedCellServiceAllocateRequest { cell } = request;

        let cell_name = cell.name.clone();
        let cell_spec = cell.into();

        let mut cells = self.cells.lock().await;

        let cell = cells.allocate(cell_name, cell_spec)?;

        Ok(CellServiceAllocateResponse {
            cell_name: cell.name().clone().to_string(),
            cgroup_v2: cell.v2().expect("allocated cell returns `Some`"),
        })
    }

    /// Frees a cell.
    ///
    /// # Arguments
    /// * `request` - A request containing CellServiceFreeRequest.
    ///
    /// # Returns
    /// A response containing CellServiceFreeResponse or a Status error.
    #[tracing::instrument(skip(self))]
    async fn free(
        &self,
        request: ValidatedCellServiceFreeRequest,
    ) -> Result<CellServiceFreeResponse> {
        let ValidatedCellServiceFreeRequest { cell_name } = request;

        info!("CellService: free() cell_name={cell_name:?}");

        let mut cells = self.cells.lock().await;

        cells.free(&cell_name)?;

        Ok(CellServiceFreeResponse::default())
    }

    #[tracing::instrument(skip(self))]
    pub(crate) async fn free_all(&self) -> Result<()> {
        let mut cells = self.cells.lock().await;

        // Attempt to gracefully free all cells
        cells.broadcast_free();

        // The cells that remain failed to shut down for some reason.
        // Forcefully kill any remaining cells that failed to shut down
        cells.broadcast_kill();

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    /// Handles a start request.
    ///
    /// # Arguments
    /// * `request` - A request containing CellServiceStartRequest.
    ///
    /// # Returns
    /// A response containing CellServiceStartResponse or a Status error.
    async fn start(
        &self,
        request: ValidatedCellServiceStartRequest,
    ) -> std::result::Result<Response<CellServiceStartResponse>, Status> {
        let ValidatedCellServiceStartRequest {
            cell_name,
            executable,
            uid,
            gid,
        } = request;

        assert!(cell_name.is_none());
        info!("CellService: start() executable={:?}", executable);

        let mut executables = self.executables.lock().await;

        // Start the executable and handle any errors
        let executable = executables
            .start(executable, uid, gid)
            .map_err(CellsServiceError::ExecutablesError)?;

        // Retrieve the process ID (PID) of the started executable
        let pid = executable
            .pid()
            .map_err(CellsServiceError::Io)?
            .expect("pid")
            .as_raw();

        // Register the stdout log channel for the executable's PID
        if let Err(e) = self
            .observe_service
            .register_sub_process_channel(
                pid,
                LogChannelType::Stdout,
                executable.stdout.clone(),
            )
            .await
        {
            warn!("failed to register stdout channel for pid {pid}: {e}");
        }

        // Register the stderr log channel for the executable's PID
        if let Err(e) = self
            .observe_service
            .register_sub_process_channel(
                pid,
                LogChannelType::Stderr,
                executable.stderr.clone(),
            )
            .await
        {
            warn!("failed to register stderr channel for pid {pid}: {e}");
        }

        let (self_uid, self_gid) =
            std::fs::metadata("/proc/self").map(|m| (m.uid(), m.gid()))?;

        Ok(Response::new(CellServiceStartResponse {
            pid,
            uid: uid.unwrap_or(self_uid),
            gid: gid.unwrap_or(self_gid),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn start_in_cell(
        &self,
        cell_name: &CellName,
        request: CellServiceStartRequest,
    ) -> std::result::Result<Response<CellServiceStartResponse>, Status> {
        do_in_cell!(self, cell_name, start, request)
    }

    #[tracing::instrument(skip(self))]
    /// Handles the stop request.
    ///
    /// # Arguments
    /// * `request` - A request containing CellServiceStopRequest.
    ///
    /// # Returns
    /// A response containing CellServiceStopResponse or a Status error.
    async fn stop(
        &self,
        request: ValidatedCellServiceStopRequest,
    ) -> std::result::Result<Response<CellServiceStopResponse>, Status> {
        let ValidatedCellServiceStopRequest { cell_name, executable_name } =
            request;

        assert!(cell_name.is_none());
        info!("CellService: stop() executable_name={:?}", executable_name,);

        let mut executables = self.executables.lock().await;

        // Retrieve the process ID (PID) of the executable to be stopped
        let pid = executables
            .get(&executable_name)
            .map_err(CellsServiceError::ExecutablesError)?
            .pid()
            .map_err(CellsServiceError::Io)?
            .expect("pid")
            .as_raw();

        // Stop the executable and handle any errors
        let _: ExitStatus = executables
            .stop(&executable_name)
            .await
            .map_err(CellsServiceError::ExecutablesError)?;

        // Remove the executable's logs from the observe service.
        if let Err(e) = self
            .observe_service
            .unregister_sub_process_channel(pid, LogChannelType::Stdout)
            .await
        {
            warn!("failed to unregister stdout channel for pid {pid}: {e}");
        }
        if let Err(e) = self
            .observe_service
            .unregister_sub_process_channel(pid, LogChannelType::Stderr)
            .await
        {
            warn!("failed to unregister stderr channel for pid {pid}: {e}");
        }

        Ok(Response::new(CellServiceStopResponse::default()))
    }

    #[tracing::instrument(skip(self))]
    async fn stop_in_cell(
        &self,
        cell_name: &CellName,
        request: CellServiceStopRequest,
    ) -> std::result::Result<Response<CellServiceStopResponse>, Status> {
        do_in_cell!(self, cell_name, stop, request)
    }

    #[tracing::instrument(skip(self))]
    pub(crate) async fn stop_all(&self) -> Result<()> {
        let mut executables = self.executables.lock().await;
        // Broadcast a stop signal to all executables
        executables.broadcast_stop().await;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn list(&self) -> Result<CellServiceListResponse> {
        let cells = self.cells.lock().await;

        // Retrieve all cells and convert them for returning
        let cells = cells
            .get_all(|x| x.try_into())
            .expect("cells doesn't error")
            .into_iter()
            .filter_map(|x| x.ok())
            .collect();

        Ok(CellServiceListResponse { cells })
    }
}

impl TryFrom<&super::cells::Cell> for CellGraphNode {
    type Error = CellsError;

    /// Converts a Cell into a CellGraphNode.
    ///
    /// # Arguments
    /// * `value` - A reference to the Cell.
    ///
    /// # Returns
    /// A result containing the CellGraphNode or an error.
    fn try_from(
        value: &super::cells::Cell,
    ) -> std::result::Result<Self, Self::Error> {
        // Extract the name and specification of the cell
        let name = value.name();
        let spec = value.spec();
        // Retrieve and convert all child cells
        let children = CellsCache::get_all(value, |x| x.try_into())?
            .into_iter()
            .filter_map(|x| x.ok())
            .collect();

        // Extract cgroup and isolation specifications
        let super::cells::CellSpec { cgroup_spec, iso_ctl } = spec;
        // Extract CPU, cpuset, and memory specifications
        let super::cells::cgroups::CgroupSpec { cpu, cpuset, memory } =
            cgroup_spec;

        Ok(Self {
            // Create a new Cell instance with the extracted specifications
            cell: Some(Cell {
                name: name.to_string(),
                cpu: cpu.as_ref().map(|x| x.into()),
                cpuset: cpuset.as_ref().map(|x| x.into()),
                memory: memory.as_ref().map(|x| x.into()),
                isolate_process: iso_ctl.isolate_process,
                isolate_network: iso_ctl.isolate_network,
            }),
            children,
        })
    }
}

impl From<&super::cells::cgroups::CpuController> for CpuController {
    fn from(value: &super::cells::cgroups::CpuController) -> Self {
        let super::cells::cgroups::CpuController { weight, max, period } =
            value.clone();

        Self {
            weight: weight.map(|x| x.into_inner()),
            max: max.map(|x| x.into_inner()),
            period,
        }
    }
}

impl From<&super::cells::cgroups::cpuset::CpusetController>
    for CpusetController
{
    fn from(value: &super::cells::cgroups::CpusetController) -> Self {
        let super::cells::cgroups::CpusetController { cpus, mems } =
            value.clone();

        Self {
            cpus: cpus.map(|x| x.into_inner()),
            mems: mems.map(|x| x.into_inner()),
        }
    }
}

impl From<&super::cells::cgroups::memory::MemoryController>
    for MemoryController
{
    fn from(value: &super::cells::cgroups::MemoryController) -> Self {
        let super::cells::cgroups::MemoryController { min, low, high, max } =
            value.clone();

        Self {
            min: min.map(|x| x.into_inner()),
            low: low.map(|x| x.into_inner()),
            high: high.map(|x| x.into_inner()),
            max: max.map(|x| x.into_inner()),
        }
    }
}

/// ### Mapping cgroup options to the Cell API
///
/// Here we *only* expose options from the CgroupBuilder
/// as our features in Aurae need them! We do not try to
/// "map" everything as much as we start with a few small
/// features and add as needed.
///
// Example builder options can be found: https://github.com/kata-containers/cgroups-rs/blob/main/tests/builder.rs
// Cgroup documentation: https://man7.org/linux/man-pages/man7/cgroups.7.html
#[tonic::async_trait]
impl cell_service_server::CellService for CellService {
    async fn allocate(
        &self,
        request: Request<CellServiceAllocateRequest>,
    ) -> std::result::Result<Response<CellServiceAllocateResponse>, Status>
    {
        // Extract the inner request from the request
        let request = request.into_inner();
        // Validate the allocate request
        let request = ValidatedCellServiceAllocateRequest::validate(
            request.clone(),
            None,
        )?;

        // return the allocated cell
        Ok(Response::new(self.allocate(request).await?))
    }

    async fn free(
        &self,
        request: Request<CellServiceFreeRequest>,
    ) -> std::result::Result<Response<CellServiceFreeResponse>, Status> {
        let request = request.into_inner();
        // Validate the free request
        let request =
            ValidatedCellServiceFreeRequest::validate(request.clone(), None)?;

        // free the cell
        Ok(Response::new(self.free(request).await?))
    }

    async fn start(
        &self,
        request: Request<CellServiceStartRequest>,
    ) -> std::result::Result<Response<CellServiceStartResponse>, Status> {
        let request = request.into_inner();

        // Execute start if cell_name is none
        if request.cell_name.is_none() {
            let request =
                ValidatedCellServiceStartRequest::validate(request, None)?;
            Ok(self.start(request).await?)
        } else {
            // We are in a parent cell, or validation will fail
            let validated = ValidatedCellServiceStartRequest::validate(
                request.clone(),
                None,
            )?;

            // Validation has succeeded, so we can make assumptions about the request and use expect
            let cell_name = validated.cell_name.expect("cell name");
            let mut request = request;
            request.cell_name = None;

            // start in the cell
            self.start_in_cell(&cell_name, request).await
        }
    }

    async fn stop(
        &self,
        request: Request<CellServiceStopRequest>,
    ) -> std::result::Result<Response<CellServiceStopResponse>, Status> {
        let request = request.into_inner();

        // Execute stop if cell_name is none
        if request.cell_name.is_none() {
            let request =
                ValidatedCellServiceStopRequest::validate(request, None)?;
            Ok(self.stop(request).await?)
        } else {
            // Validate the request is valid
            let validated = ValidatedCellServiceStopRequest::validate(
                request.clone(),
                None,
            )?;

            // Validation has succeeded, so we can make assumptions about the request and use expect
            let cell_name = validated.cell_name.expect("cell name");
            let mut request = request;
            request.cell_name = None;

            // stop the cell
            self.stop_in_cell(&cell_name, request).await
        }
    }

    /// Response with a list of cells
    ///
    /// # Arguments
    /// * `_request` - A request containing CellServiceListRequest.
    ///
    /// # Returns
    /// A response containing CellServiceListResponse or a Status error.
    async fn list(
        &self,
        _request: Request<CellServiceListRequest>,
    ) -> std::result::Result<Response<CellServiceListResponse>, Status> {
        Ok(Response::new(self.list().await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cells::cell_service::executables::ExecutableName,
        cells::cell_service::validation::{
            ValidatedCell, ValidatedCpuController, ValidatedCpusetController,
            ValidatedExecutable, ValidatedMemoryController,
        },
        logging::log_channel::LogChannel,
    };
    use crate::{AuraedRuntime, AURAED_RUNTIME};
    use iter_tools::Itertools;
    use std::ffi::OsString;
    use test_helpers::*;

    /// Test for the list function.
    #[tokio::test]
    async fn test_list() {
        skip_if_not_root!("test_list");
        skip_if_seccomp!("test_list");

        // Set the Auraed runtime for the test
        let _ = AURAED_RUNTIME.set(AuraedRuntime::default());

        // Create a new instance of CellService for testing
        let service = CellService::new(ObserveService::new(
            Arc::new(LogChannel::new(String::from("test"))),
            (None, None, None),
        ));

        // Allocate a parent cell for testing
        let parent_cell_name = format!("ae-test-{}", uuid::Uuid::new_v4());
        assert!(service
            .allocate(allocate_request(&parent_cell_name))
            .await
            .is_ok());

        // Allocate a nested cell within the parent cell for testing
        let nested_cell_name =
            format!("{}/ae-test-{}", &parent_cell_name, uuid::Uuid::new_v4());
        assert!(service
            .allocate(allocate_request(&nested_cell_name))
            .await
            .is_ok());

        // Allocate a cell without children for testing
        let cell_without_children_name =
            format!("ae-test-{}", uuid::Uuid::new_v4());
        assert!(service
            .allocate(allocate_request(&cell_without_children_name))
            .await
            .is_ok());

        // List all cells and verify the result
        let result = service.list().await;
        assert!(result.is_ok());

        let list = result.unwrap();
        assert_eq!(list.cells.len(), 2);

        // Verify the root cell names
        let mut expected_root_cell_names =
            vec![&parent_cell_name, &cell_without_children_name];
        expected_root_cell_names.sort();

        let mut actual_root_cell_names = list
            .cells
            .iter()
            .map(|c| c.cell.as_ref().unwrap().name.as_str())
            .collect_vec();
        actual_root_cell_names.sort();
        assert_eq!(actual_root_cell_names, expected_root_cell_names);

        // Verify the parent cell name in child cells.
        let parent_cell = list
            .cells
            .iter()
            .find(|p| p.cell.as_ref().unwrap().name.eq(&parent_cell_name));
        assert!(parent_cell.is_some());

        let expected_nested_cell_names = vec![&nested_cell_name];
        let actual_nested_cell_names = parent_cell
            .unwrap()
            .children
            .iter()
            .map(|c| c.cell.as_ref().unwrap().name.as_str())
            .collect_vec();
        assert_eq!(actual_nested_cell_names, expected_nested_cell_names);
    }

    #[tokio::test]
    async fn test_start_stop_free() {
        skip_if_not_root!("test_start_stop_free");
        skip_if_seccomp!("test_start_stop_free");

        let _ = AURAED_RUNTIME.set(AuraedRuntime::default());

        let service = CellService::new(ObserveService::new(
            Arc::new(LogChannel::new(String::from("test"))),
            (None, None, None),
        ));

        let cell_name = format!("ae-test-{}", uuid::Uuid::new_v4());
        assert!(service.allocate(allocate_request(&cell_name)).await.is_ok());

        let executable_name = format!("ae-exec-{}", uuid::Uuid::new_v4());
        let command = "tail -f /dev/null";
        let result =
            service.start(start_request(&executable_name, command)).await;
        assert!(result.is_ok());

        let response = result.unwrap().into_inner();
        assert!(response.pid != 0);

        assert!(service.stop(stop_request(&executable_name)).await.is_ok());

        assert!(service.free(free_request(&cell_name)).await.is_ok());
    }

    /// Helper function to create a ValidatedCellServiceAllocateRequest.
    ///
    /// # Arguments
    /// * `cell_name` - The name of the cell.
    ///
    /// # Returns
    /// A ValidatedCellServiceAllocateRequest.
    fn allocate_request(
        cell_name: &str,
    ) -> ValidatedCellServiceAllocateRequest {
        // Create a validated cell for the allocate request
        let cell = ValidatedCell {
            name: CellName::from(cell_name),
            cpu: Some(ValidatedCpuController {
                weight: None,
                max: None,
                period: None,
            }),
            cpuset: Some(ValidatedCpusetController { cpus: None, mems: None }),
            memory: Some(ValidatedMemoryController {
                min: None,
                low: None,
                high: None,
                max: None,
            }),
            isolate_process: false,
            isolate_network: false,
        };
        // Return the validated allocate request
        ValidatedCellServiceAllocateRequest { cell }
    }

    fn start_request(
        executable_name: &str,
        command: &str,
    ) -> ValidatedCellServiceStartRequest {
        let executable = ValidatedExecutable {
            name: ExecutableName::new(String::from(executable_name)),
            command: OsString::from(command),
            description: String::new(),
        };

        ValidatedCellServiceStartRequest {
            cell_name: None,
            executable,
            uid: None,
            gid: None,
        }
    }

    fn stop_request(executable_name: &str) -> ValidatedCellServiceStopRequest {
        ValidatedCellServiceStopRequest {
            cell_name: None,
            executable_name: ExecutableName::new(String::from(executable_name)),
        }
    }

    fn free_request(cell_name: &str) -> ValidatedCellServiceFreeRequest {
        // Return the validated free request
        ValidatedCellServiceFreeRequest { cell_name: CellName::from(cell_name) }
    }
}
