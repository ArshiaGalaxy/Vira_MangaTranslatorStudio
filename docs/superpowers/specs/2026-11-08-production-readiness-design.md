# Vira MangaTranslatorStudio – Production Readiness & Public Release

> Source: owner-provided specification, 2026-11-08. This is the approved design
> for the `production-readiness` branch work.

The official GitHub repository name will be:

`Vira_MangaTranslatorStudio`

Official repository URL:
https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio

My goal is to prepare the project for a public GitHub release and provide an easy-to-install Windows application for non-technical users.

## 1. Standardize Repository Name and URLs
- Update all GitHub URLs throughout the source code, documentation, configuration files, and README files.
- Fix the Auto-Updater repository URL and manifest references.
- Replace all outdated or invalid repository links.
- Ensure all internal references point to the official repository.

## 2. Fix Frontend Dependencies
- Regenerate `frontend/package-lock.json` using the official npm registry: `https://registry.npmjs.org/`.
- Remove all references to the private Liara npm mirror.
- Ensure other developers can successfully install dependencies using `npm ci` without requiring access to private infrastructure.

## 3. Review Uncommitted Project Files
- Review all currently modified and untracked Git files.
- Preserve the entire `desktop/` directory and all Tauri-related files.
- Review the Desktop Mode settings, UI changes, and Auto-Updater implementation.
- Make sure no essential files or important changes are accidentally lost.
- Ensure the repository contains all files required for a complete and functional release.

## 4. Fix the Authentication System
- Review all frontend requests to non-existent `/api/auth/*` endpoints.
- Remove or fix non-functional login and logout functionality.
- Update the documentation to accurately reflect the application's actual capabilities.
- Ensure there are no broken buttons or misleading features in the UI.

## 5. Security Improvements
- Prevent API keys and sensitive information from being exposed in public frontend code or repository files.
- Review backend configuration and environment variable handling.
- Ensure `.env` files and other sensitive information are excluded from Git.
- Keep all provider credentials properly protected.

## 6. Standardize Versions and Dependencies
- Synchronize version numbers across all project components.
- Resolve version inconsistencies between `package.json`, the backend, desktop application, and documentation.
- Review and correct `requirements.txt`.
- Ensure dependency compatibility and preserve existing working fixes.

## 7. Prepare the GitHub Repository
- Create a professional and comprehensive README.
- Provide clear installation instructions for developers.
- Provide user-friendly installation and usage instructions for the Windows application.
- Create `desktop/README.md` explaining how to build the desktop application.
- Review and improve `CONTRIBUTING.md` and other essential repository files.
- Add GitHub issue templates for bug reports and feature requests.
- Improve `.gitignore` to prevent temporary files, caches, virtual environments, and local development files from being committed.

## 8. Prepare the Windows Application
- Preserve the existing Tauri architecture.
- Prepare a distributable Windows installer using the existing Tauri capabilities.
- Package all required backend and frontend runtime dependencies with the application.
- Users must not be required to install Python, Node.js, or any development tools manually.
- Backend and frontend services must be managed and started automatically.
- Provide desktop shortcuts, Start Menu integration, and a proper uninstallation process.
- Ensure the Auto-Updater works correctly with the new official GitHub repository.
- Optimize installer size and application startup time as much as reasonably possible.

## 9. Preserve the Existing Architecture
- Keep the current Tauri + Django + Next.js architecture.
- Avoid unnecessarily rewriting components that already work.
- Preserve all existing OCR, Translation, Inpainting, and Persian Typesetting functionality.
- Do not introduce mandatory paid APIs or cloud services.
- Keep the application lightweight and suitable for ordinary computers.

## Important Constraints
- There is no need to add or restore automated tests.
- Do not modify LAN functionality or address LAN-related issues.
- Do not modify, remove, or replace the existing showcase images and manga pages. They are AI-generated.
- Do not remove existing application features.
- Do not change the architecture unless absolutely necessary.

## 10. Implementation Workflow
- First, inspect the current Git status and repository structure.
- Protect all existing uncommitted changes before making modifications.
- Create a separate Git branch named `production-readiness`.
- Identify and prioritize the existing issues.
- Implement improvements systematically, one step at a time.
- After each significant change, verify that the affected functionality still works.
- Avoid destructive operations without first preserving the original files and changes.
- At the end, provide a detailed report of completed changes and any remaining issues.

## Final Objective

The project must support two independent distribution methods:

**Path 1: Developers**
- Clone the repository from GitHub.
- Install the required dependencies.
- Run the application locally for development.
- Have clear documentation explaining the development environment and workflow.

**Path 2: Regular Users**
- Download the Windows installer from GitHub Releases.
- Install Vira like any other Windows application.
- Launch and use Vira without programming knowledge, terminal commands, or manual environment configuration.

The ultimate goal is to publish Vira MangaTranslatorStudio as a professional open-source project with a reliable Windows installation experience for non-technical users, while preserving all existing functionality and avoiding unnecessary architectural changes.
