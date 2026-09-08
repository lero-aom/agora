.PHONY: release
release:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release.ps1
