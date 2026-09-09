.PHONY: release publish-server-image
release:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release.ps1

publish-server-image:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-server-image.ps1
