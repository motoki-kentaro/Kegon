# Issue #66 Phase 0: PowerShell 報告成否の分離（レビュー案）

> [!NOTE]
> 本書は Issue #66 の初期検討過程における調査記録（設計履歴）です。確定した最終仕様は `docs/architecture/terminal.md` を参照してください。

状態: Proposed。2026-10-18 時点の調査。製品コード・PoC の実装と実機検証はまだ行っていない。
Issue: https://github.com/motoki-kentaro/Kegon/issues/66
基点: origin/main = aa354c9963ab4ac610e3a33f5e185f9dda524a9b（PR #64 を含む）。
作業場所: C:/works/kegon-66-shell-status
ブランチ: feature/66-powershell-reported-status

## 1. 確認できた現状

- assets/shell-integration/powershell.ps1 は prompt の最初で `$global:?` を保存し、元 prompt 呼出し前に失敗状態を Write-Error -ErrorAction Ignore で再現している。
- 注入フックの D は数値なし。$LASTEXITCODE から推定しない。
- src/terminal/osc.rs は D の直後の引数だけを i32 として読む。空・不正値は None。後続引数を無視する。A/B/C も後続引数を無視する。
- OscScanner は BEL/ST、分割チャンク、CAN/SUB、8 KiB 上限を扱う。
- src/terminal/integration.rs は CommandResult::{Success, Failure(i32), Unknown} に D の数値を変換する。shell_status と completion の独立した結果構造はまだない。
- VerifiedCommandEvent の Verified は状態遷移の整合性であり、通知元の認証や OS による実行成功の証明ではない。
- src/terminal/registry.rs の TerminalEntry ごとに状態機械がある。TabId は再利用されない。現在は transition の返却イベントを破棄し、状態を保持する構成。
- ChildExit は現在の実行を Unknown で閉じるが、終了後の ShellIntegration イベントを拒否する明示的な終端ガードは apply に見当たらない。#66 では遅着メタデータを受け付けないガードを検討する。
- Tooltip は CommandResult に基づく数値終了コード表示。PowerShell 報告成否欄はない。
- 既存 hook-state.ps1 は既存エラー先頭オブジェクトの同一性、件数、元 prompt が見る成否と LASTEXITCODE、入力の保存、二重 D 抑止を検査する。全エラー列・設定差・実対話との差は追加確認が必要。
- ConPTY テストは ignored。PS7 の実コマンド試験と PS7/5.1 の hook-state 試験がある。5.1 の実機試験は同梱 PSReadLine が使えない環境で A/B のみへの縮退を期待する。
- TerminalSession の Drop は終了要求。回収完了とは同義ではない。wait_for_shell_cleanup の Complete を検査する必要がある。

## 2. Phase 0 の成果と範囲

成果は「prompt 入口の値を壊さず捕捉できるか」「非曖昧に同一実行へ結び付けられるか」「既存境界・数値コードを維持できるか」の小さな PoC と採否記録。
最初はテスト用フィクスチャと純粋なパーサー／状態機械で検証し、製品フックの既定動作を変更しない。
GPU 演出、#62 の統合、#65、プロファイル変更、入力ラッパー、依存更新、Alacritty fork の変更は範囲外。

## 3. 結果契約の案

| 項目 | 意味 |
| --- | --- |
| shell_status | ReportedSuccess / ReportedFailure / Unknown。prompt 入口で PowerShell が報告した値だけ |
| osc133_exit | Option<i32>。受信した D 数値という出所を保持。外部通知であり OS 検証済みとは扱わない |
| native_exit | 対象プロセスに結び付く証拠がある場合だけ。Phase 0 には取得経路がないため None |
| completion | DObserved / MissingD / ShellExited / Desynchronized 等。観測した境界と欠落理由を記録。DObserved は全処理成功を意味しない |
| correlation | Rust 側の TabId + session generation + command_id。wire の session/sequence は別フィールドで関連付ける |

$? から 0/1 を生成しない。D;0 から shell_status を生成しない。古い LASTEXITCODE を補完しない。
例えば native exit 42 の後の cmdlet 成功は ReportedSuccess になり得るが、native_exit は None のまま。
旧 CommandResult は既存の数値コード契約を維持するか、明示的な数値専用型へ移す。shell_status を混ぜない。
#62 向けにはタブ ID を持つ型付き完了イベントを用意するが、イベント消費と描画は #62 の別作業。

## 4. 通知方式の比較と採否ゲート

| 案 | 利点 | 未解決点／判定 |
| --- | --- | --- |
| D の数値スロットの後へ名前空間付き metadata を追加 | D と status を一緒に受信でき、後着更新が不要。旧 Kegon は余分な引数を無視する | 数値なし D の空スロットを他の端末が成功 0 と解釈しない保証はない。C 側の識別子も必要。Kegon 内の互換性と他端末互換性を分けて検証する |
| 独立 metadata 通知 + 従来の C/D | A/B/C/D のバイト形式を維持できる | 番号・文法の根拠が必要。通知順序、欠落、重複、識別子の結合が増える |

D 拡張の概念例（未採用・未送信）:
`133;C;kegon.v=1;kegon.sid=<session>;kegon.seq=<n>`
`133;D;;kegon.v=1;kegon.sid=<session>;kegon.seq=<n>;kegon.ps=success`
この空スロット形式を標準対応済みとは呼ばない。

iTerm2 の公表仕様は D の数値を exit status と定義しており、この独自拡張の裏付けにはならない。
別通知の比較候補には、仕様でユーザー定義の key/value を認める OSC 1337 SetUserVar がある。ただし既存番号に新しい意味を独断で割り当てることはしない。この候補も標準 SetUserVar 文法内で Kegon 固有キーを使う案としてレビューし、任意の OSC 1337 操作を受理しない。
新規の未確認 OSC 番号、OSC 9/633 等の独自流用は行わない。

推奨順序: transport に依存しない結果契約と相関テストを先に作り、D 拡張は互換性を証明できた範囲だけ評価する。外部互換性を満たせなければ、別通知方式の採否をレビューに戻す。番号を仮採番して製品へ入れない。
旧送信者→新受信者は shell_status=Unknown、新送信者→旧 Kegon は従来の数値なし D と同じ Unknown を要求する。将来バージョン・不正値は境界処理を壊さず metadata のみ Unknown とする。

## 5. 相関と異常時の規則案

- TabId は wire の値を信用せず PTY のイベント経路から得る。wire の session/sequence を Rust の command_id と同一視しない。
- hook session ごとに識別子、非空入力ごとに単調増加 sequence。再注入・子シェル・再起動・カウンター周回は session の区切りとして処理し、古い pending を流用しない。識別子は認証用秘密ではない。
- D 拡張では C と D の session/sequence が一致し、同じタブの単一 active command がある場合だけ採用する。外部 C のみで始まったコマンドには Kegon metadata を結び付けない。
- 別通知案では begin metadata → C → result metadata → D の固定順をテストする。begin は直後の C にだけ、result はその実行の D にだけ結合する。プロトコル上の順序は PTY 読取チャンク境界に依存させない。
- pending はタブごとに最大一件。次の境界、session 変更、終了で必ず破棄。履歴から「近そうなコマンド」を探さない。
- 重複 C で command_id を増やさない。相関は曖昧になったものとしてその実行の shell_status を Unknown にする。
- metadata 重複は同一値でも異常として Unknown、競合値も Unknown。未知 version、不正 sequence、欠落、孤立 D、順序逆転も成功・失敗を補完しない。
- D 受理で結果を一度確定し、後着 metadata／重複 D で確定結果や演出イベントを更新しない。D 後に判明した異常は診断として扱う。
- A/B による D 欠落、shell exit、タブ閉鎖で pending を消し Unknown。終了を受理した session への後続通知は破棄する。
- Ctrl+C は観測から確実に区別できる範囲のみ中断扱い。$? だけで中断と通常失敗を識別しない。対応付け不能時は Unknown を許容する。
- metadata の文字集合・フィールド数・長さを厳格化し、PoC は 512 byte 以下の固定的な小さい payload を対象にする。既存 OSC 全体の 8 KiB 上限は維持する。
- OSC は偽装可能。session ID が一致してもシェル本人／OS 由来の認証とは説明しない。

## 6. PowerShell 意味論と非干渉の検証

prompt 最初の読み取りを維持し、その前に helper、ログ、時刻取得、文字列生成、条件判定を挿入しない。metadata は保存済み boolean から生成する。

PS7 以前では (...) / $(...) / @(...) によって $? が True に戻る場合があり、PS7 で挙動が変わる。cmdlet、関数、pipeline、複合文も含め、両バージョンの結果を一律の「全処理成功」に正規化しない。構文エラーは実行されず $? を変えない場合もあり、前の値を新しい実行の成功証明にしない。

Write-Error -ErrorAction Ignore は文書上 $Error に記録しない。現行実装を直ちに不具合と決め付けず、フックなし／現行フック／候補フックの同一ケースを比較する。
比較対象は元 prompt 入口の $? と LASTEXITCODE、$Error 全列の順序・件数・オブジェクト同一性、出力文字列、呼出回数、副作用、エラーストリーム。prompt 呼出し後の値も比較し、元 prompt 自身が起こす変化と区別する。
ErrorActionPreference、PSDefaultParameterValues、StrictMode、関数名 shadowing、prompt の例外等で非干渉を主張できる範囲を調べる。公開 API で保証できなければ非対応範囲として記録し、大きな書換えはレビューへ戻す。
本番コードで $Error.Clear/Remove を使った帳尻合わせ、$LASTEXITCODE の書換え、reflection による内部状態変更は行わない。

PS5.1 の fixture 成功は実 ConPTY で C/D が取得できる証明ではない。PSReadLine 非対応は A/B と CWD の既存動作を維持し shell_status=Unknown。モジュールの追加導入やユーザー設定変更はしない。

## 7. 安全なテスト範囲

1. 純粋な parser/state-machine tests: D / D;0 / D;42、数値と成否の独立、未知 version、重複キー、不正長、不正 UTF-8、全分割位置、BEL/ST、CAN/SUB、欠落・重複・逆順、2 タブ交互、session 切替、終了後通知、設定 OFF。
2. 隔離した -NoProfile 子プロセスの differential fixture: 上記の prompt 状態保存。まず既存 hook-state 回帰を基準化。ユーザーの profile を読まず書かない。
3. 明示起動する ignored ConPTY tests: PS7/5.1 のバージョンと PSReadLine capability を記録。cmd /c exit 0 と 42、cmdlet 成否、未実行分岐、先行失敗→後続成功、pipeline、括弧式、redirect 準備失敗、引数評価失敗、空入力、繰返し prompt、Ctrl+C、shell exit。
4. 独立した scratch directory を使い、失敗用パスはそこで生成。実機ケースは直列、各ケース 10 秒・suite 全体 120 秒の deadline 案。待機ごとに時間枠を延長しない。イベント件数・ログ量も上限化する。
5. 成否・panic・timeout の全経路で session を drop し、wait_for_shell_cleanup(3 秒) の Complete を明示 assert。外側 watchdog を使う場合もそのテストが所有する子プロセスだけを回収し、プロセス名による一括 kill はしない。子孫回収を保証できない harness では長時間／再帰 spawn ケースを走らせない。
6. Kegon GUI は起動・キー送信しない。GUI Tooltip の最終確認は PO が手動で行う。

## 8. 実装スライス案

| 順番 | 対象 | 完了条件 |
| --- | --- | --- |
| 0A | test fixture と非干渉の調査記録 | PS7/5.1 の基準値・差・非対応範囲が分かる。製品送信形式はそのまま |
| 0B | transport 文法の比較 fixture、osc.rs の局所 PoC | 数値非捏造、旧 Kegon 互換、外部互換の根拠と限界を記録して方式を決定 |
| 1 | integration.rs / osc.rs / session.rs / registry.rs | 独立した結果型、タブ別相関、終端ガード、単一完了イベント。異常系 tests PASS |
| 2 | powershell.ps1 と既存 shell integration tests | 選定済み通知方式の実装、非干渉・PS7 実 ConPTY 成否・5.1 縮退が PASS |
| 3 | app.rs / i18n / architecture 記録 | 「PowerShell報告成否」と「数値終了コード（OSC報告）」を別表示。Unknown と未実行を区別。#62 は型契約のみ |

Phase 0 の採否記録をレビューした後に製品実装へ進む。
Issue 完了時は cargo fmt --check、cargo check --locked、cargo clippy --all-targets --locked -- -D warnings、cargo test --locked、cargo build --release --locked を実行し、ignored 実機試験と PO 手動 GUI 確認は別欄で報告する。
現時点でこれらを PASS とする証拠はない。

## 9. 保全と確認範囲

- #62 worktree/branch: b4d211204917bae44a052e8bcbe2b23fb2cda00e、clean を確認。
- #63 worktree/branch: 1f6782c8394234246063210510fca3dbae8e3181、clean を確認。
- stash: a11c931251b21d702cbdd1ced34de35aa6db2d58 を維持。
- Alacritty: Cargo.toml / Cargo.lock の 758174acb2bf184cc55f177a56b8cf3292967f9f 固定を維持。fork の checkout 自体は操作していない。
- 今回は設計書のみ追加。製品コード、依存、profile、sources/ は変更しない。commit / push / PR / merge / GUI 自動キー送信なし。

## 一次資料

- Microsoft: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_automatic_variables?view=powershell-7.5
- Microsoft: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_commonparameters?view=powershell-7.5
- iTerm2（D と SetUserVar）: https://iterm2.com/documentation-escape-codes.html
- WezTerm（OSC 133 と追加シェル状態）: https://wezterm.org/shell-integration.html
- terminal-wg semantic-prompts 原文は今回取得できていない。外部端末の独自拡張対応は未検証。
