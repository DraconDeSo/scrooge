$ErrorActionPreference = 'Stop'

$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  url64bit       = 'https://github.com/DraconDeSo/scrooge/releases/download/v0.1.1/Scrooge_0.1.1_x64-setup.exe'
  checksum64     = 'b8c6b4173b6ecde436fdecd7a4f6f96e97f481b71ad6465bc0c5caa6a8323bd1'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
}

Install-ChocolateyPackage @packageArgs
