/* -------------------------------------------------------------------------- *\
 *          Apache 2.0 License Copyright © 2022-2023 The Aurae Authors        *
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
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::prelude::DirEntryExt;
use tracing::warn;

/// Used for looking up cgroup paths by inode number
struct CgroupCache {
    root: OsString,
    cache: HashMap<u64, OsString>,
}

impl CgroupCache {
    pub fn new(root: OsString) -> Self {
        Self { root, cache: HashMap::new() }
    }

    pub fn get(&mut self, ino: u64) -> Option<OsString> {
        if let Some(path) = self.cache.get(&ino) {
            Some(path.clone())
        } else {
            self.refresh_cache();
            self.cache.get(&ino).cloned()
        }
    }

    fn refresh_cache(&mut self) {
        fs::read_dir(&self.root)
            .unwrap_or_else(|_| panic!("could not read from {:?}", self.root))
            .for_each(|res| match res {
                Ok(dir_entry) => {
                    _ = self
                        .cache
                        .insert(dir_entry.ino(), dir_entry.file_name());
                }
                Err(e) => {
                    warn!("could not read from {:?}: {}", self.root, e);
                }
            });
    }
}

#[cfg(test)]
mod test {
    use std::fs::File;

    use super::*;

    #[test]
    fn get_must_return_none_when_file_doesnt_exist() {
        let mut cache = CgroupCache::new(OsString::from("/tmp"));

        assert_eq!(cache.get(123), None);
    }

    #[test]
    fn get_must_return_file_for_ino() {
        let mut cache = CgroupCache::new(OsString::from("/tmp"));

        let file_name1 = OsString::from(uuid::Uuid::new_v4().to_string());
        let ino1 = create_file(&file_name1);

        let file_name2 = OsString::from(uuid::Uuid::new_v4().to_string());
        let ino2 = create_file(&file_name2);

        assert!(cache.get(ino1).is_some());
        assert!(cache
            .get(ino1)
            .expect("should not happen")
            .eq_ignore_ascii_case(file_name1));

        assert!(cache.get(ino2).is_some());
        assert!(cache
            .get(ino2)
            .expect("should not happen")
            .eq_ignore_ascii_case(file_name2));
    }

    fn create_file(file_name: &OsString) -> u64 {
        let _file = File::create(format!(
            "/tmp/{}",
            file_name
                .to_ascii_lowercase()
                .to_str()
                .expect("couldn't convert filename")
        ))
        .expect("couldn't create file");
        let dir_entry = fs::read_dir("/tmp")
            .expect("tmp dir entries")
            .find(|e| {
                println!("{:?}", e.as_ref().expect("").file_name());
                e.as_ref()
                    .expect("file")
                    .file_name()
                    .eq_ignore_ascii_case(file_name)
            })
            .expect("couldn't find file")
            .expect("dir entry");
        dir_entry.ino()
    }
}
