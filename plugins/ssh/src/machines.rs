// Minuteman - a fast, Ranger-inspired terminal file manager
// Copyright (C) 2026  Davi Oliveira Gonçalves
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The sessions this plugin holds: one per machine, shared by every path on it and by every
//! request thread, and the sorting of a request's path into the machine it names.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use plugin_api::VfsFault;
use shared::ssh_path::{Location, Remote, Target};

use crate::executor::Executor;
use crate::link::{ConnectOptions, Link};

pub(crate) struct Machines {
    executor: Arc<Executor>,
    options: ConnectOptions,
    links: Mutex<HashMap<Target, Arc<Link>>>,
}

impl Machines {
    pub(crate) fn new(options: ConnectOptions) -> io::Result<Self> {
        Ok(Self {
            executor: Arc::new(Executor::new()?),
            options,
            links: Mutex::new(HashMap::new()),
        })
    }

    fn link(&self, target: &Target) -> Arc<Link> {
        let mut links = self.links.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(links.entry(target.clone()).or_insert_with(|| {
            Arc::new(Link::new(
                target.clone(),
                self.options.clone(),
                Arc::clone(&self.executor),
            ))
        }))
    }

    /// The session and address that `path` names.
    ///
    /// # Errors
    ///
    /// `NotFound` for a path that climbs out of the machine's root, and `Unsupported` for one that
    /// is not an `ssh://` address, which this plugin does not serve.
    pub(crate) fn route(&self, path: &Path) -> Result<(Arc<Link>, Remote), VfsFault> {
        match Remote::locate(path) {
            Ok(Location::Remote(remote)) => Ok((self.link(remote.target()), remote)),
            Ok(Location::Beyond) => Err(VfsFault::NotFound),
            Ok(Location::Local) => Err(VfsFault::Unsupported {
                reason: "this is not a path on a remote machine".to_string(),
            }),
            Err(error) => Err(VfsFault::Io {
                kind: plugin_api::IoKind::InvalidInput,
                message: error.to_string(),
            }),
        }
    }

    /// Both paths' session and addresses, for a request that needs them on one machine.
    ///
    /// # Errors
    ///
    /// As [`Self::route`], and `Unsupported` when the two name different machines.
    pub(crate) fn route_pair(
        &self,
        a: &Path,
        b: &Path,
    ) -> Result<(Arc<Link>, Remote, Remote), VfsFault> {
        let (link, a) = self.route(a)?;
        let (_, b) = self.route(b)?;
        if a.target() != b.target() {
            return Err(VfsFault::Unsupported {
                reason: "not on the same machine".to_string(),
            });
        }
        Ok((link, a, b))
    }
}
