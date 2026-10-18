# Kegon #66 Phase 0A: PowerShell 報告成否の非干渉 PoC 検証報告書

> [!NOTE]
> 本書は Issue #66 の検討過程における非干渉 PoC 検証記録（設計履歴）です。確定した最終仕様は `docs/architecture/terminal.md` を参照してください。

- **Issue**: [#66](https://github.com/motoki-kentaro/Kegon/issues/66)
- **Worktree**: `C:\works\kegon-66-shell-status`
- **Branch**: `feature/66-powershell-reported-status`
- **Base Commit**: `aa354c9963ab4ac610e3a33f5e185f9dda524a9b` (origin/main)
- **Date**: 2026-10-18
- **Phase**: Phase 0A（PowerShell報告成否の非干渉PoC）

---

## 1. 目的とスコープ

Kegon の OSC 133 Shell Integration（#63 / PR #64）において、PowerShell が `prompt` 入口で報告する直前の実行成否 `$?` を、**ユーザーの既存シェル環境・エラー履歴・元プロンプトに一切の副作用を与えることなく捕捉・引継ぎできること** を独立検証する。

本 Phase 0A では、通信プロトコル（OSC番号やメタデータ構文）および Rust の製品コード（状態機械・パーサー・UI等）は一切変更せず、隔離された PowerShell 7（pwsh 7.6.7）および Windows PowerShell 5.1（5.1.26100.9587）環境での振る舞いを網羅的に比較・検証した。

---

## 2. 検証ハーネスの設計と「ハーネス自体の非干渉性」の保証

PowerShell において `$?` は **直前のあらゆるステートメントの成否** によって上書きされる自動変数である。
検証ハーネス自身がログ出力、条件判定、変数代入、関数呼び出しなどのステートメントを実行してしまうと、観測対象であるコマンドの `$?` が書き換えられてしまい、誤った検証結果（偽陽性・偽陰性）を生じさせる。

本 PoC ハーネス（`assets/shell-integration/tests/phase0a-harness.ps1`）では、以下の厳格な設計を採用した。

### (1) コマンドと prompt 呼出しの直結
```powershell
$sbText = "$CommandText; `$global:TestPromptOutput = prompt"
$sb = [scriptblock]::Create($sbText)
& $sb
```
- コマンド実行から `prompt` 関数突入までの間に、中間ステートメントを一切挟まない。
- 終端例外（terminating exception）発生時も、`catch` ブロックの第1行目で即座に `prompt` を呼び出す。

### (2) prompt 入口での即座の保存
```powershell
function global:prompt {
    # 最初の行でいかなる補助処理も行わず、直ちに $? と $LASTEXITCODE を退避
    $capturedSuccess = $global:?
    $capturedExitCode = $global:LASTEXITCODE
    ...
```

### (3) 元 prompt への完全な状態復元
```powershell
    # 失敗状態（$? == $false）の時だけ Write-Error -ErrorAction Ignore を発行
    if (-not $capturedSuccess) {
        Microsoft.PowerShell.Utility\Write-Error 'failure' -ErrorAction Ignore
    }

    # 元 prompt を呼出し、元 prompt が認識する成否を記録
    $orig = & $global:__KegonState.OriginalPrompt
```
- `Microsoft.PowerShell.Utility\Write-Error` を完全修飾することで、ユーザーによる同名関数/エイリアスのシャドーイングを回避。
- `-ErrorAction Ignore` により、`$Error` コレクションへのエラーレコード追加を 100% 抑止（`$ErrorActionPreference = 'Stop'` や `$PSDefaultParameterValues`、`Set-StrictMode` が設定されていても停止・汚染しないことを実証）。

---

## 3. 実測結果（33 ケースの網羅比較）

PowerShell 7.6.7 および Windows PowerShell 5.1.26100.9587 における実測比較表（`assets/shell-integration/tests/phase0a-comparison.md` より抜粋）：

| Case | Category | Description / Command | PS 7 `$?` | WinPS 5.1 `$?` | 挙動一致 | PS 7 元prompt一致 | WinPS 元prompt一致 | PS 7 LASTEXIT | WinPS LASTEXIT |
|---|---|---|---|---|---|---|---|---|---|
| **1.1** | 1. 正常なcmdlet | `Get-Date \| Out-Null` | True | True | **MATCH** | OK | OK | - | - |
| **1.2** | 1. 正常なcmdlet | `Write-Output "kegon_test" \| Out-Null` | True | True | **MATCH** | OK | OK | - | - |
| **1.3** | 1. 正常なcmdlet | `Get-Process -Id $PID \| Out-Null` | True | True | **MATCH** | OK | OK | - | - |
| **2.1** | 2. 失敗するcmdlet | `Get-Item NonExistent -ErrorAction SilentlyContinue` | False | False | **MATCH** | OK | OK | - | - |
| **2.2** | 2. 失敗するcmdlet | `Get-Item NonExistent` (Continue) | False | False | **MATCH** | OK | OK | - | - |
| **2.3** | 2. 失敗するcmdlet | `Write-Error probe -ErrorAction SilentlyContinue` | False | False | **MATCH** | OK | OK | - | - |
| **3.1** | 3. 正常終了ネイティブ | `cmd.exe /c exit 0` | True | True | **MATCH** | OK | OK | 0 | 0 |
| **3.2** | 3. 正常終了ネイティブ | `cmd.exe /c "echo hello > nul"` | True | True | **MATCH** | OK | OK | 0 | 0 |
| **4.1** | 4. 非ゼロ終了ネイティブ | `cmd.exe /c exit 42` | False | False | **MATCH** | OK | OK | 42 | 42 |
| **4.2** | 4. 非ゼロ終了ネイティブ | `cmd.exe /c exit 1` | False | False | **MATCH** | OK | OK | 1 | 1 |
| **5.1** | 5. リダイレクト失敗 | `Write-Output fail > "Z:\NonExistent\out.txt"` | False | False | **MATCH** | OK | OK | 1 | 1 |
| **5.2** | 5. リダイレクト失敗 | `cmd.exe /c exit 0 > "Z:\NonExistent\out.txt"` | False | False | **MATCH** | OK | OK | 1 | 1 |
| **6.1** | 6. 未実行if分岐 | `Write-Output success; if ($false) { Write-Error bad }` | True | True | **MATCH** | OK | OK | 1 | 1 |
| **6.2** | 6. 未実行if分岐 | `cmd.exe /c exit 0; if ($false) { cmd.exe /c exit 42 }` | True | True | **MATCH** | OK | OK | 0 | 0 |
| **6.3** | 6. 未実行if分岐 | `Write-Error prior_fail; if ($false) { Write-Output unreached }` | False | False | **MATCH** | OK | OK | 0 | 0 |
| **7.1** | 7. 複合文 失敗後成功 | `Write-Error fail; Write-Output ok` | True | True | **MATCH** | OK | OK | 0 | 0 |
| **7.2** | 7. 複合文 失敗後成功 | `cmd.exe /c exit 42; Write-Output ok` | True | True | **MATCH** | OK | OK | 42 | 42 |
| **7.3** | 7. 複合文 失敗後成功 | `cmd.exe /c exit 42; cmd.exe /c exit 0` | True | True | **MATCH** | OK | OK | 0 | 0 |
| **7.4** | 7. 複合文 成功後失敗 | `cmd.exe /c exit 0; cmd.exe /c exit 42` | False | False | **MATCH** | OK | OK | 42 | 42 |
| **8.1** | 8. パイプライン | `1..3 \| Where-Object { $_ -gt 1 }` | True | True | **MATCH** | OK | OK | 42 | 42 |
| **8.2** | 8. パイプライン | `cmd.exe /c exit 42 \| Out-Null` | False | False | **MATCH** | OK | OK | 42 | 42 |
| **8.3** | 8. パイプライン | `cmd.exe /c exit 42 \| cmd.exe /c exit 0` | False | False | **MATCH** | OK | OK | 0 | 0 |
| **8.4** | 8. パイプライン | `Write-Error pipe_err \| Out-Null` | False | False | **MATCH** | OK | OK | 0 | 0 |
| **8.5** | 8. パイプライン | `1..3 \| ForEach-Object { if ($_ -eq 2) { Write-Error err } }` | True | True | **MATCH** | OK | OK | 0 | 0 |
| **9.1** | 9. 括弧式・部分式 | `(cmd.exe /c exit 42)` | **False** | **True** | **DIFF** | OK | OK | 42 | 42 |
| **9.2** | 9. 括弧式・部分式 | `$(cmd.exe /c exit 42)` | **False** | **True** | **DIFF** | OK | OK | 42 | 42 |
| **9.3** | 9. 括弧式・部分式 | `@(cmd.exe /c exit 42)` | **False** | **True** | **DIFF** | OK | OK | 42 | 42 |
| **9.4** | 9. 括弧式・部分式 | `(Get-Item NonExistent -ErrorAction SilentlyContinue)` | **False** | **True** | **DIFF** | OK | OK | 42 | 42 |
| **9.5** | 9. 括弧式・部分式 | `$(Get-Item NonExistent -ErrorAction SilentlyContinue)` | **False** | **True** | **DIFF** | OK | OK | 42 | 42 |
| **9.6** | 9. 括弧式・部分式 | `@(Get-Item NonExistent -ErrorAction SilentlyContinue)` | **False** | **True** | **DIFF** | OK | OK | 42 | 42 |
| **10.1** | 10. 空Enter/中断 | 空Enter（入力なしのまま `prompt` 呼出し） | True | True | **MATCH** | OK | OK | 42 | 42 |
| **10.2** | 10. 空Enter/中断 | 失敗直後の空Enter（`prompt` 2回連続呼出し） | True | True | **MATCH** | OK | OK | 42 | 42 |
| **10.3** | 10. 空Enter/中断 | 終端例外 / パイプライン中断（Ctrl+C / throw） | False | False | **MATCH** | OK | OK | 42 | 42 |

---

## 4. 観測項目ごとの検証結果

### (1) 元 prompt への状態引継ぎの整合性（`SuccessMatched`）
- **全 33 ケースで 100% 一致**（`PS 7: 33/33 PASS`, `WinPS 5.1: 33/33 PASS`）。
- Kegon が prompt 入口で捕捉した成否（`$capturedSuccess`）と、元 prompt が受け取った成否（`$global:OriginalPromptSawSuccess`）の間に一切の乖離が存在しないことを証明。

### (2) `$LASTEXITCODE` の非干渉性（`ExitCodeMatched`）
- **全 33 ケースで 100% 一致**。
- Kegon のフック処理（CWD取得、OSCマーカー構築、Write-Error実行など）は、`$LASTEXITCODE` を一切書き換えない。

### (3) `$Error` 履歴の保全（`InitialErrorPreserved`）
- テストセッション開始時に投入したユーザーエラーオブジェクト（`$global:InitialUserError`）が、全 33 ケース実行後も参照同一性（`[object]::ReferenceEquals`）を保ったまま配列内に残存することを確認。
- エラーを発生させないコマンド（正常cmdlet、正常ネイティブ、未実行分岐等）において、Kegon のフックがエラーを追加することは一切ない（`ErrorsAdded = 0`）。
- 失敗復元用 `Write-Error -ErrorAction Ignore` は、`$Error` 配列に要素を追加しない。

### (4) `$ErrorActionPreference` の不変性
- 全ケースにおいて、テスト前の設定値（`Continue`）がそのまま維持された。

### (5) 既存 OSC 133 A/B/C/D への影響
- 通常コマンド実行時は、`D`（境界終了）、`A`（プロンプト開始）、`OSC 7`（CWD）、`B`（入力開始）の完全なシーケンスが出力された。
- 空Enter（Case 10.1 / 10.2）では `Executing = $false` であるため、孤立 `D` は出力されず、`A` と `B` のみが安全に出力された。

---

## 5. 発見された重要な意味論的制限（Semantics & Limits）

### 制限 1：括弧式・部分式における PS 7 と WinPS 5.1 の根本的な挙動差（Category 9）
- **現象**: `(cmd.exe /c exit 42)` や `$(Get-Item NonExistent)` のように括弧・部分式・配列式で囲まれた場合：
  - **PowerShell 7**: 内部コマンドの失敗をそのまま反映し、`$? = False` となる。
  - **Windows PowerShell 5.1**: 括弧・部分式自体の評価（式の完了）が成功とみなされ、**`$?` が一律 `True` にリセットされる**。
- **結論**: Windows PowerShell 5.1 における `$?` は言語仕様上の制限により、式評価構文での失敗を拾えない場合がある。この挙動は Kegon のフックによるものではなくシェル自身の仕様であるため、Kegon はこれを無理に正規化（捏造）せず、**「シェルが報告した値（Reported）」** としてそのまま扱うことが妥当である。

### 制限 2：複合文での先行失敗の上書き（Category 7）
- **現象**: `cmd.exe /c exit 42; Write-Output ok` のように複数文が連続して実行された場合、`prompt` 入口の `$?` は **最後のステートメント（`Write-Output ok`）の成否（`True`）** を報告する。一方で `$LASTEXITCODE` は `42` のまま残る。
- **結論**: `$?` は「コマンドライン全体の全処理成功」を保証するものではなく、「プロンプト直前に完了した最終実行の成否」である。Issue #66 の結果契約（`ReportedSuccess` / `ReportedFailure`）は、まさにこの性質を正しくモデル化している。

### 制限 3：リダイレクト準備失敗時のネイティブ未実行（Category 5）
- **現象**: `cmd.exe /c exit 0 > "Z:\InvalidPath\out.txt"` のように無効なパスへリダイレクトした場合、ネイティブコマンド自体は起動されない。`$?` は `False` となるが、`$LASTEXITCODE` は以前の古い値（例えば直前のネイティブコマンドの終了コード）を保持したままになる。
- **結論**: ネイティブコマンド名が含まれていても、プロセスが起動されたとは限らない。OSC 133 D の数値 exit code と、シェル報告成否（`shell_status`）を厳密に分離して管理する方針の正しさが実証された。

### 制限 4：未実行 if 分岐の `$?` 不変性（Category 6）
- **現象**: `if ($false) { ... }` のようにどの分岐も実行されなかった場合、`if` 構文自体は `$?` を変更しない（直前の `$?` がそのまま通過する）。

### 制限 5：パイプライン内ループでの例外（Category 8）
- **現象**: `1..3 | ForEach-Object { if ($_ -eq 2) { Write-Error err } }` では、ループ途中で非終了エラーが発生してもパイプライン全体は最後まで走り切るため、最終的な `$?` は `True` となる。

---

## 6. Phase 0B への進捗判定と残存リスク

### 判定：**Phase 0B への進行を推奨（PASS with notes）**

1. **非干渉性の立証完了**:
   - `prompt` 入口での `$?` 捕捉は、`Write-Error 'failure' -ErrorAction Ignore` と組み合わせることで、元プロンプト、`$LASTEXITCODE`、`$Error` 配列、`$ErrorActionPreference` に一切の悪影響を与えないことが証明された。
2. **意味論的境界の明確化**:
   - 複合文、リダイレクト失敗、括弧式（PS 5.1固有挙動）における `$?` の振る舞いが網羅的にカタログ化され、Dopamine Mode や Tooltip に提供するメタデータが「OSの確証付き成功」ではなく「シェル報告成否（`ReportedSuccess` / `ReportedFailure`）」であるべき根拠が固まった。

### 残存リスクと Phase 0B での検証課題
1. **通知プロトコルの選定（Phase 0Bの主題）**:
   - `OSC 133;D` の拡張構文（例: 名前空間付き引数）と別通知（OSC 1337 SetUserVar 等）について、既存の他端末（Windows Terminal, VS Code, iTerm2 等）や旧 Kegon との相互運用性を検証する必要がある。
2. **PSReadLine 非対応環境（WinPS 5.1）の縮退**:
   - ConPTY 上の Windows PowerShell 5.1 では PSReadLine が動作しない場合があり、C/D マーカー自体が発行されない。その際は安全に `shell_status = Unknown` へ縮退することを確認する。
