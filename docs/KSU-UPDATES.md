# KSU module updates

A (`ommega`) and B (`ommegaclient_b`) module.prop files point at the latest
GitHub release assets:

- https://github.com/Andrea-lyz/ommega/releases/latest/download/ommega-a-update.json
- https://github.com/Andrea-lyz/ommega/releases/latest/download/ommega-b-update.json

Pushing a tag matching `v*` runs the Build workflow. The release job downloads
the verified bundle, adds stable `ommega-a-latest.zip` / `ommega-b-latest.zip`
aliases, writes both update JSON files with the zip versionCode, and publishes
the GitHub release as latest. KSU Manager then sees the update on its next
refresh.

The currently installed module must first be replaced once with a zip that
contains updateJson. After that, future updates can use KSU Manager directly.
