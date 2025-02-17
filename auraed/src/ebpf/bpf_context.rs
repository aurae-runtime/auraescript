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
    kprobe::KProbeProgram, perf_buffer_reader::PerfBufferReader,
    perf_event_broadcast::PerfEventBroadcast, tracepoint::TracepointProgram,
    BpfFile,
};

use aya::Ebpf;
use tracing::warn;

// This is critical to maintain the memory presence of the
// loaded bpf object.
// This specific BPF object needs to persist up to lib.rs such that
// the rest of the program can access this scope.
pub struct BpfContext(Vec<Ebpf>);

impl BpfContext {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn load_and_attach_tracepoint_program<TProgram, TEvent>(
        &mut self,
    ) -> Result<PerfEventBroadcast<TEvent>, anyhow::Error>
    where
        TProgram:
            BpfFile + TracepointProgram<TEvent> + PerfBufferReader<TEvent>,
        TEvent: Clone + Send + 'static,
    {
        match TProgram::load() {
            Ok(mut bpf_handle) => {
                TProgram::load_and_attach(&mut bpf_handle)?;
                let perf_events = TProgram::read_from_perf_buffer(
                    &mut bpf_handle,
                    TProgram::PERF_BUFFER,
                );
                self.0.push(bpf_handle);
                perf_events
            }
            Err(e) => {
                warn!(
                    "Error loading tracepoint program {}: {}",
                    TProgram::PROGRAM_NAME,
                    e
                );
                Err(e.into())
            }
        }
    }

    pub fn load_and_attach_kprobe_program<TProgram, TEvent>(
        &mut self,
    ) -> Result<PerfEventBroadcast<TEvent>, anyhow::Error>
    where
        TProgram: BpfFile + KProbeProgram<TEvent> + PerfBufferReader<TEvent>,
        TEvent: Clone + Send + 'static,
    {
        match TProgram::load() {
            Ok(mut bpf_handle) => {
                TProgram::load_and_attach(&mut bpf_handle)?;
                let perf_events = TProgram::read_from_perf_buffer(
                    &mut bpf_handle,
                    TProgram::PERF_BUFFER,
                );
                self.0.push(bpf_handle);
                perf_events
            }
            Err(e) => {
                warn!(
                    "Error loading kprobe program {}: {}",
                    TProgram::PROGRAM_NAME,
                    e
                );
                Err(e.into())
            }
        }
    }
}
