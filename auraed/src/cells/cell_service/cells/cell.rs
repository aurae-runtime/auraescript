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
    cgroups::Cgroup, nested_auraed::NestedAuraed, CellName, CellSpec, Cells,
    CellsCache, CellsError, Result,
};
use client::AuraeSocket;
use tracing::info;

// TODO https://github.com/aurae-runtime/aurae/issues/199 &&
//      aurae.io/signals, which is more accurate
// TODO nested auraed should proxy (bus) POSIX signals to child executables

macro_rules! do_free {
    (
        $self:ident,
        $nested_auraed_call:ident($($nested_auraed_call_arg:ident),*),
        $($children_call:ident($($children_call_arg:ident),*)),*
    ) => {{
        if let CellState::Allocated { cgroup, nested_auraed, children } =
            &mut $self.state
        {
            $(children.$children_call($($children_call_arg),*));*;

            let _exit_status = nested_auraed
                .$nested_auraed_call($($nested_auraed_call_arg),*)
                .map_err(|e| {
                    CellsError::FailedToKillCellChildren {
                        cell_name: $self.cell_name.clone(),
                        source: e,
                    }
                })?;

            cgroup.delete().map_err(|e| CellsError::FailedToFreeCell {
                cell_name: $self.cell_name.clone(),
                source: e,
            })?;
        }

        // set cell state to freed, independent of the current state
        $self.state = CellState::Freed;

        Ok(())
    }};
}

// We should not be able to change a cell after it has been created.
// You must free the cell and create a new one if you want to change anything about the cell.
// In order to facilitate that immutability:
// NEVER MAKE THE FIELDS PUB (OF ANY KIND)
#[derive(Debug)]
pub struct Cell {
    cell_name: CellName,
    spec: CellSpec,
    state: CellState,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum CellState {
    Unallocated,
    Allocated { cgroup: Cgroup, nested_auraed: NestedAuraed, children: Cells },
    Freed,
}

impl Cell {
    pub fn new(cell_name: CellName, cell_spec: CellSpec) -> Self {
        Self { cell_name, spec: cell_spec, state: CellState::Unallocated }
    }

    /// Creates the underlying cgroup.
    /// Does nothing if [Cell] has been previously allocated.
    // Here is where we define the "default" cgroup parameters for Aurae cells
    pub fn allocate(&mut self) -> Result<()> {
        let CellState::Unallocated = &self.state else {
            return Ok(());
        };

        let name = self.cell_name.leaf().to_string();

        let mut auraed = NestedAuraed::new(name, self.spec.iso_ctl.clone())
            .map_err(|e| CellsError::FailedToAllocateCell {
                cell_name: self.cell_name.clone(),
                source: e,
            })?;

        let pid = auraed.pid();

        let cgroup = match Cgroup::new(
            self.cell_name.clone(),
            self.spec.cgroup_spec.clone(),
            pid,
        ) {
            Ok(cgroup) => cgroup,
            Err(e) => {
                let _best_effort = auraed.kill();
                return Err(CellsError::AbortedAllocateCell {
                    cell_name: self.cell_name.clone(),
                    source: e,
                });
            }
        };

        if let Err(e) = cgroup.add_task(pid) {
            let _best_effort = auraed.kill();
            let _best_effort = cgroup.delete();

            return Err(CellsError::AbortedAllocateCell {
                cell_name: self.cell_name.clone(),
                source: e,
            });
        }

        info!("Attach nested Auraed pid {} to cgroup {}", pid, self.cell_name);

        self.state = CellState::Allocated {
            cgroup,
            nested_auraed: auraed,
            children: Cells::new(self.cell_name.clone()),
        };

        Ok(())
    }

    /// Broadcasts a graceful shutdown signal to all [NestedAuraed] and
    /// deletes the underlying cgroup and all descendants.
    ///
    /// The [Cell::state] will be set to [CellState::Freed] regardless of it's state prior to this call.
    ///
    /// A [Cell] should never be reused once in the [CellState::Freed] state.
    pub fn free(&mut self) -> Result<()> {
        do_free!(self, shutdown(), broadcast_free())
    }

    /// Sends a [SIGKILL] to the [NestedAuraed], and deletes the underlying cgroup.
    /// The [Cell::state] will be set to [CellState::Freed] regardless of it's state prior to this call.
    /// A [Cell] should never be reused once in the [CellState::Freed] state.
    pub fn kill(&mut self) -> Result<()> {
        do_free!(self, kill(), broadcast_kill())
    }

    pub fn client_socket(&self) -> Result<AuraeSocket> {
        let CellState::Allocated { nested_auraed, .. } = &self.state else {
            return Err(CellsError::CellNotAllocated {
                cell_name: self.cell_name.clone(),
            });
        };

        Ok(nested_auraed.client_socket.clone())
    }

    /// Returns the [CellName] of the [Cell]
    pub fn name(&self) -> &CellName {
        &self.cell_name
    }

    pub fn spec(&self) -> &CellSpec {
        &self.spec
    }

    /// Returns [None] if the [Cell] is not allocated.
    pub fn v2(&self) -> Option<bool> {
        let CellState::Allocated { cgroup, .. } = &self.state else {
            return None;
        };

        Some(cgroup.v2())
    }
}

impl CellsCache for Cell {
    fn allocate(
        &mut self,
        cell_name: CellName,
        cell_spec: CellSpec,
    ) -> Result<&Cell> {
        let CellState::Allocated { children, .. } = &mut self.state else {
            return Err(CellsError::CellNotAllocated {
                cell_name: self.cell_name.clone(),
            });
        };

        children.allocate(cell_name, cell_spec)
    }

    fn free(&mut self, cell_name: &CellName) -> Result<()> {
        let CellState::Allocated { children, .. } = &mut self.state else {
            return Err(CellsError::CellNotAllocated {
                cell_name: self.cell_name.clone(),
            });
        };

        children.free(cell_name)
    }

    fn get<F, R>(&mut self, cell_name: &CellName, f: F) -> Result<R>
    where
        F: Fn(&Cell) -> Result<R>,
    {
        let CellState::Allocated { children, .. } = &mut self.state else {
            return Err(CellsError::CellNotAllocated {
                cell_name: self.cell_name.clone(),
            });
        };

        children.get(cell_name, f)
    }

    fn get_all<F, R>(&self, f: F) -> Result<Vec<Result<R>>>
    where
        F: Fn(&Cell) -> Result<R>,
    {
        let CellState::Allocated { children, .. } = &self.state else {
            return Err(CellsError::CellNotAllocated {
                cell_name: self.cell_name.clone(),
            });
        };

        children.get_all(f)
    }

    fn broadcast_free(&mut self) {
        let CellState::Allocated { children, .. } = &mut self.state else {
            return;
        };

        children.broadcast_free()
    }

    fn broadcast_kill(&mut self) {
        let CellState::Allocated { children, .. } = &mut self.state else {
            return;
        };

        children.broadcast_kill()
    }
}

impl Drop for Cell {
    /// During normal behavior, cells are freed before being dropped,
    /// but cache reconciliation may result in a drop in other circumstances.
    /// Here we have a chance to clean up, no matter the circumstance.
    fn drop(&mut self) {
        // We use kill here to be aggressive in cleaning up if anything has been left behind.
        let _best_effort = self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuraedRuntime, AURAED_RUNTIME};
    use test_helpers::*;

    #[test]
    fn test_cant_unfree() {
        skip_if_not_root!("test_cant_unfree");
        // Docker's seccomp security profile (https://docs.docker.com/engine/security/seccomp/) blocks clone
        skip_if_seccomp!("test_cant_unfree");

        let _ = AURAED_RUNTIME.set(AuraedRuntime::default());

        let cell_name = CellName::random_for_tests();
        let mut cell = Cell::new(cell_name, CellSpec::new_for_tests());
        assert!(matches!(cell.state, CellState::Unallocated));

        cell.allocate().expect("failed to allocate");
        assert!(matches!(cell.state, CellState::Allocated { .. }));

        cell.free().expect("failed to free");
        assert!(matches!(cell.state, CellState::Freed));

        // Calling allocate again should do nothing
        cell.allocate().expect("failed to allocate 2");
        assert!(matches!(cell.state, CellState::Freed));
    }
}
