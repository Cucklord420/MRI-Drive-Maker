# CI dependency pins

## PowerShell static analysis

The Windows PowerShell 5.1 static-analysis workflow uses PSScriptAnalyzer **1.25.0**. The module manifest for this release requires PowerShell 5.1. The workflow downloads the official PowerShell Gallery package, checks its pinned SHA-512 digest, verifies the Authenticode signatures on the module manifest and script, and requires the reviewed Microsoft signing certificate before importing the module. It logs the verified version and digests, then keeps the existing `Error`-severity gate. It does not cache the package or use modules from the repository checkout.

Reviewed upstream sources (2026-10-18):

- Release: https://github.com/PowerShell/PSScriptAnalyzer/releases/tag/1.25.0
- Gallery package: https://www.powershellgallery.com/packages/PSScriptAnalyzer/1.25.0
- Package SHA-512: `e7fb573262e604bab29914ae62921d24b97c2f88b519b41277b403ef6ce163f14b7d1b36dc73449a68e5ae5a8d9502891b10c264131fd1813adf29bf131c1663`
- Authenticode signer certificate SHA-256: `b41c444f8cbd49d1b27cc2c76e0f3fb042bf9970b6b6f6b57fc8976514b03952`

### Updating the pin

1. Review a specific upstream PSScriptAnalyzer release and confirm its manifest supports Windows PowerShell 5.1.
2. Download the package from both the official PowerShell Gallery and the matching upstream GitHub release. Confirm the bytes match, calculate SHA-512, and compare it with the Gallery package metadata.
3. Inspect the Authenticode signatures on `PSScriptAnalyzer.psd1` and `PSScriptAnalyzer.psm1`. Record the signer certificate's SHA-256 fingerprint and verify that Windows reports each signature as valid.
4. Update the version, package digest, and signer fingerprint together in `.github/workflows/static-analysis.yml` and this document. Review any diagnostics introduced by the new analyzer release.
5. Run the workflow on its supported Windows PowerShell 5.1 runner and confirm the logged version, package digest, signer fingerprint, and `Error`-severity result.

Keep the dependency-update review in a normal pull request. The workflow does not prepare or modify MRI media; its result is not evidence of physical USB or firmware behavior.
