use std::{collections::HashMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    app::AppError,
    reccmp::{
        Address, ReccmpBuildTarget, ReccmpBuildYaml, ReccmpProjectYaml, ReccmpUserTarget,
        ReccmpUserYaml,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Compile,
    Reccmp,
    Stackcmp(Address),
    Datacmp,
    Roadmap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TabKind {
    Listing,
    Diff { func_name: String },
    Stackcmp { address: Address, func_name: String },
    Roadmap,
    Vtable { name: String },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConfigFiles {
    pub build: PathBuf,
    pub project: PathBuf,
    pub user: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BuildConfig {
    pub cwd: PathBuf,
    pub cmd: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AppConfig {
    pub files: ConfigFiles,
    pub build: BuildConfig,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DisassemblySettings {
    pub relative_jump: bool,
    pub resolve_symbols: bool,
    pub use_roadmap: bool,
    pub use_roadmap_func_sizes: bool,
    pub resolve_global_offsets: bool,
    pub display_global_offsets_hex: bool,
}

impl Default for DisassemblySettings {
    fn default() -> Self {
        Self {
            relative_jump: true,
            resolve_symbols: true,
            use_roadmap: true,
            use_roadmap_func_sizes: false,
            resolve_global_offsets: true,
            display_global_offsets_hex: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WatchSettings {
    pub active: bool,
    pub debounce_ms: u64,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            active: true,
            debounce_ms: 150,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub recent_projects: Vec<PathBuf>,
    pub current_project: Option<PathBuf>,
    pub current_target: Option<String>,
    pub reccmp: Option<PathBuf>,
    pub stackcmp: Option<PathBuf>,
    pub datacmp: Option<PathBuf>,
    pub roadmap: Option<PathBuf>,
    pub no_library: bool,
    pub disassembly: DisassemblySettings,
    pub watch: WatchSettings,
    pub show_sidebar: bool,
    pub dock_state: Option<egui_dock::DockState<TabKind>>,
    pub restore_on_startup: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            current_project: None,
            current_target: None,
            reccmp: which::which("reccmp-reccmp").ok(),
            stackcmp: which::which("reccmp-stackcmp").ok(),
            datacmp: which::which("reccmp-datacmp").ok(),
            roadmap: which::which("reccmp-roadmap").ok(),
            no_library: false,
            disassembly: DisassemblySettings::default(),
            watch: WatchSettings::default(),
            show_sidebar: true,
            dock_state: None,
            restore_on_startup: true,
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolState {
    pub compiling: Option<u64>,
    pub reccmp: Option<u64>,
    pub roadmap: Option<u64>,
    pub datacmp: Option<u64>,
    pub stackcmp: HashMap<Address, u64>,
    pub cancelling: bool,
}

impl ToolState {
    pub fn is_busy(&self) -> bool {
        self.compiling.is_some()
            || self.reccmp.is_some()
            || self.roadmap.is_some()
            || self.datacmp.is_some()
            || !self.stackcmp.is_empty()
    }

    pub fn is_idle(&self) -> bool {
        !self.is_busy() && !self.cancelling
    }

    pub fn update_cancelling(&mut self) {
        if !self.is_busy() {
            self.cancelling = false;
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub config: AppConfig,
    pub dir: PathBuf,
    pub build_yml: ReccmpBuildYaml,
    pub project_yml: ReccmpProjectYaml,
    pub user_yml: ReccmpUserYaml,
    pub tool_state: ToolState,
    pub target: String,
    pub is_watching: bool,
}

impl Project {
    pub fn new(config: AppConfig, dir: PathBuf) -> Result<Self, AppError> {
        let build_yml_path = dir.join(&config.files.build);
        let build_yml_str =
            fs::read_to_string(&build_yml_path).map_err(|e| AppError::ReadFile {
                path: build_yml_path.clone(),
                source: e,
            })?;
        let build_yml = yaml_serde::from_str(&build_yml_str).map_err(|e| AppError::YamlRead {
            path: build_yml_path,
            source: e,
        })?;

        let project_yml_path = dir.join(&config.files.project);
        let project_yml_str =
            fs::read_to_string(&project_yml_path).map_err(|e| AppError::ReadFile {
                path: project_yml_path.clone(),
                source: e,
            })?;
        let project_yml: ReccmpProjectYaml =
            yaml_serde::from_str(&project_yml_str).map_err(|e| AppError::YamlRead {
                path: project_yml_path,
                source: e,
            })?;

        let user_yml_path = dir.join(&config.files.user);
        let user_yml_str = fs::read_to_string(&user_yml_path).map_err(|e| AppError::ReadFile {
            path: user_yml_path.clone(),
            source: e,
        })?;
        let user_yml = yaml_serde::from_str(&user_yml_str).map_err(|e| AppError::YamlRead {
            path: user_yml_path,
            source: e,
        })?;

        Ok(Self {
            config,
            dir,
            build_yml,
            project_yml,
            user_yml,
            tool_state: ToolState::default(),
            target: String::new(),
            is_watching: false,
        })
    }

    pub fn get_user_target(&self, target: &str) -> Result<&ReccmpUserTarget, AppError> {
        let target = self
            .user_yml
            .targets
            .get(target)
            .ok_or_else(|| AppError::TargetMissing {
                target: target.to_owned(),
                path: self.dir.join(&self.config.files.user),
            })?;
        Ok(target)
    }

    pub fn get_build_target(&self, target: &str) -> Result<&ReccmpBuildTarget, AppError> {
        let target = self
            .build_yml
            .targets
            .get(target)
            .ok_or_else(|| AppError::TargetMissing {
                target: target.to_owned(),
                path: self.dir.join(&self.config.files.build),
            })?;
        Ok(target)
    }
}
