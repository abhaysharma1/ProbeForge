# Inert fixture repository

This directory is inert data used to test read-only Git ingestion. It contains
package manifests with install/build/test/start scripts, an executable shell
script, and a Makefile. ProbeForge must never install dependencies, run a build,
run tests, start an application, or execute any script from this repository.

`scripts/pwn.js` writes `fixture-executed.txt` if it is ever run. Tests assert
that the marker never appears in the temporary workspace.
