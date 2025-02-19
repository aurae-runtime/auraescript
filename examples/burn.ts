#!/usr/bin/env auraescript
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

import * as helpers from "../auraescript/gen/helpers.ts";
import * as runtime from "../auraescript/gen/cells.ts";

// TODO can't sleep, no setTimeout and no equivalent in Deno.core as far as I can tell
// function sleep(n: number) {
//     return new Promise(resolve => setTimeout(resolve, n));
// }

const cells = new runtime.CellServiceClient();

const cellName = "cpu-burn-room";

let allocated = await cells.allocate(<runtime.CellServiceAllocateRequest>{
    cell: runtime.Cell.fromPartial({
        name: cellName,
        cpu: runtime.CpuController.fromPartial({
            weight: 100, // Percentage of CPUs

            // 40% -- 2/5 of one core
            max: 400_000, // usec

            // // 200% -- 2 cpu cores
            // max: 2_000_000, // usec

            // period: 1_000_000 // usec, hardcoded default
        }),
    })
});
helpers.print('Allocated:', allocated)

// TODO if we had a fs api and injected script name, this could be based on those...
const devRoot = '/home/jcorbin/aurae';

const nodeBurn = runtime.Executable.fromPartial({
    command: `/usr/bin/bash -c 'node ${devRoot}/examples/burn.js >${devRoot}/node-burn.out 2>&1'`,
    description: "Burn CPU in Node.JS while monitoring runtime lag",
    name: "node-burn"
});

// // NOTE: build exe with `go build -o go-burn examples/burn.go`
// const goBurn = runtime.Executable.fromPartial({
//     command: `/usr/bin/bash -c 'GOMAXPROCS=4 ${devRoot}/go-burn >${devRoot}/go-burn.out 2>&1'`,
//     description: "Burn CPU in Go while monitoring runtime lag",
//     name: "go-burn"
// });

let started = await cells.start(<runtime.CellServiceStartRequest>{
    cellName,
    executable: nodeBurn,
    // executable: goBurn,
});

helpers.print('Started:', started);

// NOTE: to run this, it's best to start a fresh auraed like:
//
//   $ sudo -E auraed -v
//
// Then run this auraescript:
//
//   $ auraescript examples/burn.ts
//
// Confirm that you've got a successuflly started pid above, maybe take a moment to look it it in (h)top. Then run:
//
//   $ watch -n 0.1 cat /sys/fs/cgroup/cpu-burn-room/cpu.stat
//
// And confirm that you see nr_throttled going up.
//
// Then take a look at the redirected log file:
//
//   $ less node-burn.out
//
// You should see either a constant rate of outlier times (for the node
// program), or an entirely skewed box (75%-ile) value in the case of a heavily
// over-subscribed Go program.
//
// Once done, simply stop the auraed, or its cpu-burn-room cell within it if you prefer.
//
// For the Go version, you'll need to swap commented-out code around above to
// use the goBurn executable, build the executable as noted above, and maybe
// also change the allocated max CPU value to be ast least 2 cores. Output can then be seen using:
//
//   $ less go-burn.out

// TODO if we had the ability to sleep, we could sit here and poll
// /sys/fs/cgroup/${cellName}/cpu.stat

// TODO harvest stdout/err log streams; instead we use bash to redirect into a file above for now

// await sleep(20_000); // let it run for 20s

// TODO oncw we can do the above, we can then stop and free
//  await cells.stop(<runtime.CellServiceStopRequest>{
//     cellName,
//     executableName: "node-burn",
// });
//  await cells.free(<runtime.CellServiceFreeRequest>{
//     cellName
// });
