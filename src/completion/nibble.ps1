# nibble completion for PowerShell. Load it from $PROFILE with: nibble completion powershell | Out-String | Invoke-Expression
Register-ArgumentCompleter -Native -CommandName nibble -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $words = @($commandAst.CommandElements | Where-Object { $_.Extent.EndOffset -lt $cursorPosition } | ForEach-Object { $_.ToString() })
    $out = @(& nibble __complete "--current=$wordToComplete" -- @words 2>$null)
    if ($out.Count -eq 0 -or $out[0] -ne ':none') { return }
    $out | Select-Object -Skip 1 | ForEach-Object {
        $value, $help = $_ -split "`t", 2
        if (-not $help) { $help = $value }
        [System.Management.Automation.CompletionResult]::new($value, $value, 'ParameterValue', $help)
    }
}
