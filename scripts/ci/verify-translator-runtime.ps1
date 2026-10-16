param(
    [Parameter(Mandatory = $true)][string]$RuntimeDir,
    [Parameter(Mandatory = $true)][string]$EngineTag,
    [string]$Ct2Source,
    [string]$Ct2Install,
    [string]$WorkerBuild
)
$ErrorActionPreference = 'Stop'
# 构建时收集依赖、许可及来源；消费已发布包时仅验证，绝不重写 manifest。
if ($Ct2Source) {
    python "$PSScriptRoot/translator-runtime.py" prepare $RuntimeDir --tag $EngineTag --ct2-source $Ct2Source --ct2-install $Ct2Install --worker-build $WorkerBuild
    if ($LASTEXITCODE -ne 0) { throw '引擎依赖/许可归档失败。' }
}
python "$PSScriptRoot/translator-runtime.py" verify $RuntimeDir --tag $EngineTag
if ($LASTEXITCODE -ne 0) { throw '引擎 manifest、依赖或 health 验证失败。' }
