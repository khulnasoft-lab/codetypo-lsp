//! Workspace and routing state management for Codetypo-LSP.

use anyhow::anyhow;
use matchit::Router;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url, WorkspaceFolder};

use crate::codetypo::Instance;

#[derive(Default)]
/// State for the Codetypo-LSP backend, including severity, config, workspace folders, and router.
pub(crate) struct BackendState<'s> {
    pub severity: Option<DiagnosticSeverity>,
    pub config: Option<PathBuf>,
    pub workspace_folders: Vec<WorkspaceFolder>,
    pub router: Router<crate::codetypo::Instance<'s>>,
}

impl BackendState<'_> {
    /// Sets the workspace folders and updates the router.
    pub(crate) fn set_workspace_folders(
        &mut self,
        workspace_folders: Vec<WorkspaceFolder>,
    ) -> anyhow::Result<(), anyhow::Error> {
        self.workspace_folders = workspace_folders;
        self.update_router()?;
        Ok(())
    }

    /// Updates the workspace folders by adding and removing, then updates the router.
    pub(crate) fn update_workspace_folders(
        &mut self,
        added: Vec<WorkspaceFolder>,
        removed: Vec<WorkspaceFolder>,
    ) -> anyhow::Result<(), anyhow::Error> {
        self.workspace_folders.extend(added);
        if !removed.is_empty() {
            self.workspace_folders.retain(|x| !removed.contains(x));
        }
        self.update_router()?;
        Ok(())
    }

    /// Updates the internal router for workspace folders.
    pub(crate) fn update_router(&mut self) -> anyhow::Result<(), anyhow::Error> {
        self.router = Router::new();
        for folder in self.workspace_folders.iter() {
            let path = folder
                .uri
                .to_file_path()
                .map_err(|_| anyhow!("Cannot convert uri {} to file path", folder.uri))?;
            let route = format!("{}{}", url_path_sanitised(&folder.uri), "/{*p}");
            self.router
                .insert_instance(&route, &path, self.config.as_deref())?;
        }

        // add low priority catch all route used for files outside the workspace, or
        // when there is no workspace folder
        #[cfg(windows)]
        for drive in crate::windows::get_drives() {
            let route = format!("/{}%3A/{{*p}}", &drive);
            self.router.insert_instance(
                &route,
                &PathBuf::from(format!("{}:\\", &drive)),
                self.config.as_deref(),
            )?;
        }

        #[cfg(not(windows))]
        {
            let route = "/{*p}";
            self.router
                .insert_instance(route, &PathBuf::from("/"), self.config.as_deref())?;
        }

        Ok(())
    }
}

/// Extension trait for inserting Codetypo instances into the router.
trait RouterExt {
    /// Inserts a new Codetypo instance into the router for the given route and path.
    fn insert_instance(
        &mut self,
        route: &str,
        path: &Path,
        config: Option<&Path>,
    ) -> anyhow::Result<(), anyhow::Error>;
}

impl RouterExt for Router<Instance<'_>> {
    // convenience method to insert a new CodetypoCli into the router
    // implemented as an extension trait to avoid interprocedural conflicts
    fn insert_instance(
        &mut self,
        route: &str,
        path: &Path,
        config: Option<&Path>,
    ) -> anyhow::Result<(), anyhow::Error> {
        tracing::debug!("Adding route {} for path {}", route, path.display());
        let instance = Instance::new(path, config)?;
        self.insert(route, instance)?;
        Ok(())
    }
}

pub fn url_path_sanitised(url: &Url) -> String {
    // windows paths (eg: /C:/Users/..) may not be percent-encoded by some clients
    // and therefore contain colons, see
    // https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#uri
    //
    // and because matchit treats colons as a wildcard we need to strip them
    url.path().replace(':', "%3A")
}
