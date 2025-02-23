use crate::{env, plugins};
use heck::ToKebabCase;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crate::backend::{ABackend, Backend, BackendList, BackendType};
use crate::cache::{CacheManager, CacheManagerBuilder};
use crate::cli::args::BackendArg;
use crate::config::{Config, SETTINGS};
use crate::dirs;
use crate::install_context::InstallContext;
use crate::plugins::vfox_plugin::VfoxPlugin;
use crate::plugins::{Plugin, PluginType};
use crate::toolset::{ToolVersion, Toolset};
use crate::ui::multi_progress_report::MultiProgressReport;

#[derive(Debug)]
pub struct VfoxBackend {
    ba: BackendArg,
    plugin: Box<VfoxPlugin>,
    remote_version_cache: CacheManager<Vec<String>>,
    exec_env_cache: CacheManager<BTreeMap<String, String>>,
    pathname: String,
}

impl Backend for VfoxBackend {
    fn get_type(&self) -> BackendType {
        BackendType::Vfox
    }

    fn get_plugin_type(&self) -> PluginType {
        PluginType::Vfox
    }

    fn fa(&self) -> &BackendArg {
        &self.ba
    }

    fn _list_remote_versions(&self) -> eyre::Result<Vec<String>> {
        self.remote_version_cache
            .get_or_try_init(|| {
                let (vfox, _log_rx) = self.plugin.vfox();
                self.ensure_plugin_installed()?;
                let versions = self
                    .plugin
                    .runtime()?
                    .block_on(vfox.list_available_versions(&self.pathname))?;
                Ok(versions
                    .into_iter()
                    .rev()
                    .map(|v| v.version)
                    .collect::<Vec<String>>())
            })
            .cloned()
    }

    fn install_version_impl(&self, ctx: &InstallContext) -> eyre::Result<()> {
        self.ensure_plugin_installed()?;
        let (vfox, log_rx) = self.plugin.vfox();
        thread::spawn(|| {
            for line in log_rx {
                // TODO: put this in ctx.pr.set_message()
                info!("{}", line);
            }
        });
        self.plugin.runtime()?.block_on(vfox.install(
            &self.pathname,
            &ctx.tv.version,
            ctx.tv.install_path(),
        ))?;
        Ok(())
    }

    fn list_bin_paths(&self, tv: &ToolVersion) -> eyre::Result<Vec<PathBuf>> {
        let path = self
            ._exec_env(tv)?
            .iter()
            .find(|(k, _)| k.to_uppercase() == "PATH")
            .map(|(_, v)| v.to_string())
            .unwrap_or("bin".to_string());
        Ok(env::split_paths(&path).collect())
    }

    fn exec_env(
        &self,
        _config: &Config,
        _ts: &Toolset,
        tv: &ToolVersion,
    ) -> eyre::Result<BTreeMap<String, String>> {
        self._exec_env(tv).cloned()
    }
}

impl VfoxBackend {
    pub fn list() -> eyre::Result<BackendList> {
        Ok(plugins::INSTALLED_PLUGINS
            .iter()
            .filter(|(_, pt)| matches!(pt, PluginType::Vfox))
            .map(|(d, _)| {
                Arc::new(Self::from_arg(
                    d.file_name().unwrap().to_string_lossy().into(),
                )) as ABackend
            })
            .collect())
    }

    pub fn from_arg(ba: BackendArg) -> Self {
        let pathname = ba.short.to_kebab_case();
        let plugin_path = dirs::PLUGINS.join(&pathname);
        Self {
            remote_version_cache: CacheManagerBuilder::new(
                ba.cache_path.join("remote_versions.msgpack.z"),
            )
            .with_fresh_duration(SETTINGS.fetch_remote_versions_cache())
            .with_fresh_file(dirs::DATA.to_path_buf())
            .with_fresh_file(plugin_path.to_path_buf())
            .with_fresh_file(ba.installs_path.to_path_buf())
            .build(),
            exec_env_cache: CacheManagerBuilder::new(ba.cache_path.join("exec_env.msgpack.z"))
                .with_fresh_file(dirs::DATA.to_path_buf())
                .with_fresh_file(plugin_path.to_path_buf())
                .with_fresh_file(ba.installs_path.to_path_buf())
                .build(),
            plugin: Box::new(VfoxPlugin::new(pathname.clone())),
            ba,
            pathname,
        }
    }

    fn _exec_env(&self, tv: &ToolVersion) -> eyre::Result<&BTreeMap<String, String>> {
        self.exec_env_cache.get_or_try_init(|| {
            self.ensure_plugin_installed()?;
            let (vfox, _log_rx) = self.plugin.vfox();
            Ok(self
                .plugin
                .runtime()?
                .block_on(vfox.env_keys(&self.pathname, &tv.version))?
                .into_iter()
                .map(|envkey| (envkey.key, envkey.value))
                .collect())
        })
    }

    fn ensure_plugin_installed(&self) -> eyre::Result<()> {
        self.plugin
            .ensure_installed(&MultiProgressReport::get(), false)
    }
}
