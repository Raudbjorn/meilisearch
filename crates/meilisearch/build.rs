fn main() {
    #[cfg(feature = "mini-dashboard")]
    mini_dashboard::setup_mini_dashboard().expect("Could not load the mini-dashboard assets");
}

#[cfg(feature = "mini-dashboard")]
mod mini_dashboard {
    use std::env;
    use std::path::PathBuf;

    use static_files::resource_dir;

    pub fn setup_mini_dashboard() -> anyhow::Result<()> {
        let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

        // Use local dashboard build from monorepo
        let dashboard_dir = cargo_manifest_dir.join("../../dashboard/build");

        if !dashboard_dir.exists() {
            anyhow::bail!(
                "Dashboard build directory not found at {:?}. \
                 Run 'npm run build' in the dashboard/ directory first.",
                dashboard_dir
            );
        }

        resource_dir(&dashboard_dir).build()?;

        Ok(())
    }
}
