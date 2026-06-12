//! Actionable setup guidance for TypeScript SDK scaffolding during `kalam init`.

use std::path::Path;

use super::package_manager::PackageManager;
use crate::workflow::project::guidance::bullet_list;

pub fn format_detected_managers(installed: &[PackageManager]) -> String {
    if installed.is_empty() {
        "none".to_string()
    } else {
        installed.iter().map(|manager| manager.as_str()).collect::<Vec<_>>().join(", ")
    }
}

pub fn init_no_templates() -> String {
    format!(
        "no built-in TypeScript templates are available in this CLI build.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Reinstall or upgrade the Kalam CLI package you are using",
            "Report the issue if you built kalam-cli from source without bundled templates",
        ])
    )
}

pub fn init_missing_package_managers(installed: &[PackageManager]) -> String {
    let detected = format_detected_managers(installed);
    format!(
        "no JavaScript package manager was found on PATH.\n\n\
         TypeScript projects need npm, pnpm, yarn, or bun to install SDK dependencies.\n\n\
         Detected on PATH: {detected}\n\n\
         How to fix:\n{}\n\n\
         After installing one, verify it works:\n\
         npm --version\n\
         pnpm --version",
        bullet_list(&[
            "Install Node.js from https://nodejs.org (includes npm)",
            "Or install pnpm: https://pnpm.io/installation",
            "Or install bun: https://bun.sh",
            "Open a new terminal so PATH updates are picked up",
            "Rerun: kalam init --yes --languages typescript",
            "Or choose explicitly once installed: kalam init --package-manager pnpm",
        ])
    )
}

pub fn init_package_manager_not_on_path(
    requested: PackageManager,
    installed: &[PackageManager],
) -> String {
    let detected = format_detected_managers(installed);
    let requested_name = requested.as_str();
    let mut fixes = vec![
        format!("Install {requested_name} and open a new terminal"),
        format!("Or choose another installed manager: kalam init --package-manager <name>"),
        format!("Detected on PATH: {detected}"),
    ];
    if !installed.is_empty() {
        let alt = installed.iter().map(|manager| manager.as_str()).collect::<Vec<_>>().join(", ");
        fixes.insert(
            1,
            format!(
                "Use one that is already installed ({alt}): kalam init --package-manager {alt}"
            ),
        );
    }
    fixes.push(format!("Finish manually after init: cd <project> && {requested_name} install"));

    format!(
        "{requested_name} was not found on PATH.\n\nHow to fix:\n{}",
        bullet_list(&fixes)
    )
}

pub fn init_package_install_failed(
    manager: PackageManager,
    project_dir: &Path,
    exit_status: &str,
    stdout: &str,
    stderr: &str,
) -> String {
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let mut hints = diagnose_package_install_output(&combined);
    if hints.is_empty() {
        hints.push(
            "Read the command output above for the package manager's error message".to_string(),
        );
        hints.push("Try running the install command manually in the project directory".to_string());
    }

    format!(
        "{} {} failed with exit status {exit_status} in '{}'.\n\n\
         Command output:\n\
         stdout:\n{}\n\
         stderr:\n{}\n\n\
         How to fix:\n{}\n\n\
         Project files were already created. Finish setup manually:\n{}",
        manager.as_str(),
        manager.install_args().join(" "),
        project_dir.display(),
        indent_block(stdout.trim()),
        indent_block(stderr.trim()),
        bullet_list(&hints),
        bullet_list(&[
            format!("cd {}", project_dir.display()),
            format!("{} install", manager.as_str()),
            "kalam dev".to_string(),
        ])
    )
}

pub fn init_package_install_spawn_failed(
    manager: PackageManager,
    project_dir: &Path,
    error: &str,
) -> String {
    format!(
        "could not run {} install in '{}' ({error}).\n\n\
         How to fix:\n{}\n\n\
         Project files were already created. Finish setup manually:\n{}",
        manager.as_str(),
        project_dir.display(),
        bullet_list(&[
            format!("Verify the manager works: {} --version", manager.as_str()),
            "Open a new terminal if you installed Node.js or pnpm recently".to_string(),
            format!("Run manually: cd {} && {} install", project_dir.display(), manager.as_str()),
        ]),
        bullet_list(&[
            format!("cd {}", project_dir.display()),
            format!("{} install", manager.as_str()),
            "kalam dev".to_string(),
        ])
    )
}

fn indent_block(text: &str) -> String {
    if text.is_empty() {
        return "  (empty)".to_string();
    }
    text.lines().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n")
}

fn diagnose_package_install_output(combined: &str) -> Vec<String> {
    let mut hints = Vec::new();

    if combined.contains("eacces") || combined.contains("permission denied") {
        hints.push(
            "Permission error: rerun the terminal as Administrator (Windows) or fix directory ownership (macOS/Linux)".to_string(),
        );
    }
    if combined.contains("enotfound")
        || combined.contains("etimedout")
        || combined.contains("network")
        || combined.contains("fetch failed")
    {
        hints.push(
            "Network/registry error: check internet access, VPN, proxy, and corporate firewall rules for the npm registry".to_string(),
        );
        hints.push(
            "Retry manually with a clean cache: npm cache clean --force (or the equivalent for your package manager)".to_string(),
        );
    }
    if combined.contains("ebadengine")
        || combined.contains("unsupported engine")
        || combined.contains("node version")
    {
        hints.push(
            "Node.js version mismatch: upgrade Node.js to the current LTS release from https://nodejs.org".to_string(),
        );
    }
    if combined.contains("cannot find module '@kalamdb")
        || combined.contains("404 not found")
        || combined.contains("notarget")
    {
        hints.push(
            "Package resolution failed: verify package.json was created and your registry can reach the @kalamdb scope".to_string(),
        );
    }
    if combined.contains("peer dep") || combined.contains("eresolve") {
        hints.push(
            "Dependency resolution conflict: try `npm install --legacy-peer-deps` or use the package manager recorded in kalam.toml".to_string(),
        );
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_missing_package_managers_lists_install_options() {
        let message = init_missing_package_managers(&[]);
        assert!(message.contains("no JavaScript package manager"));
        assert!(message.contains("How to fix:"));
        assert!(message.contains("nodejs.org"));
    }

    #[test]
    fn init_package_manager_not_on_path_suggests_installed_alternatives() {
        let message =
            init_package_manager_not_on_path(PackageManager::Npm, &[PackageManager::Pnpm]);
        assert!(message.contains("npm was not found"));
        assert!(message.contains("pnpm"));
        assert!(message.contains("--package-manager"));
    }

    #[test]
    fn diagnose_package_install_output_detects_network_failures() {
        let hints = diagnose_package_install_output("getaddrinfo enotfound registry.npmjs.org");
        assert!(hints.iter().any(|hint| hint.contains("Network")));
    }

    #[test]
    fn init_package_install_failed_includes_manual_recovery_steps() {
        let temp = tempfile::TempDir::new().unwrap();
        let message =
            init_package_install_failed(PackageManager::Pnpm, temp.path(), "1", "", "ERR!");
        assert!(message.contains("pnpm install failed"));
        assert!(message.contains("kalam dev"));
    }
}
