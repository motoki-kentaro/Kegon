# Kegon UI strings: ja-JP.
#
# Messages missing here fall back to en-US at runtime, but the test suite
# expects this locale to be complete.

## Tab bar (tooltips)

tab-bar-settings-tooltip = 設定

settings-language-label = 表示言語
settings-language-system = システム設定に従う
settings-cli-override-note = このセッションでは --locale により言語が上書きされています。
settings-theme-label = テーマ
settings-ui-font-label = UI フォント
settings-ui-font-system = システム標準
settings-ui-font-choose = 選択...
settings-ui-font-reset = リセット
settings-ui-font-not-installed = { $family } (未インストール)
settings-terminal-font-label = ターミナル フォント
settings-terminal-font-default = 標準
settings-terminal-font-choose = 選択...
settings-terminal-font-reset = リセット
settings-terminal-font-not-installed = { $family } (未インストール)


## Terminal tab strip

terminal-tab-default-title = ターミナル
terminal-new-tab-tooltip = 新しいタブ
terminal-close-tab-tooltip = ターミナルを閉じる
terminal-process-exited = プロセスがコード { $code } で終了しました。
terminal-tooltip-shell-integration = Shell Integration
terminal-tooltip-status-active = 有効
terminal-tooltip-status-active-prompt-only = 有効 (プロンプトのみ)
terminal-tooltip-status-waiting = 待機中
terminal-tooltip-status-disabled = 無効
terminal-tooltip-status-unsupported = 非対応
terminal-tooltip-status-unavailable = 利用不可
terminal-tooltip-command-state = コマンド状態
terminal-tooltip-command-state-unknown = 不明
terminal-tooltip-command-state-prompt = プロンプト
terminal-tooltip-command-state-editing = 編集中
terminal-tooltip-command-state-executing = 実行中
terminal-tooltip-command-state-finished = 完了
terminal-tooltip-shell-reported-status = シェル報告成否
terminal-tooltip-status-success = 成功
terminal-tooltip-status-failure = 失敗
terminal-tooltip-status-unknown = 不明
terminal-tooltip-osc-exit-code = OSC報告終了コード
terminal-tooltip-exit-code-none = —

## Settings tab

settings-tab-title = 設定
settings-tab-description = 変更はすぐに反映され、自動的に保存されます。
settings-category-general-title = 一般
settings-category-general-description = アプリケーション全体に関わる設定です。
settings-category-appearance-title = 外観
settings-category-appearance-description = テーマと、インターフェースで使うフォントです。
settings-category-terminal-title = ターミナル
settings-category-terminal-description = ターミナルセッションの表示方法です。
settings-font-size-label = フォントサイズ
settings-font-size-description = ターミナル文字の大きさ(pt)です。下の設定がなければ、セルの幅と高さもこれに合わせて変わります。
settings-font-size-default = 9.75 pt(13 px)
settings-cell-height-label = 行の高さ
settings-cell-height-description = セルの高さを、フォントサイズに対する倍率で指定します(0.5〜3.0)。未設定ならフォントに従います。
settings-cell-height-default = フォントに従う
settings-cell-width-label = セルの幅
settings-cell-width-description = セルの幅を、フォントサイズに対する倍率で指定します(0.2〜2.0)。未設定ならフォントに従います。
settings-cell-width-default = フォントに従う
settings-metric-current = 現在値: { $value }
settings-metric-default-value = 既定: { $value }
settings-metric-pending = 未適用
settings-metric-apply = 適用
settings-metric-reset = リセット
settings-metric-range-error = { $min } から { $max } の数値を入力してください。
settings-metric-stored-invalid = settings.toml の値({ $value })は無効なため無視されています。新しい値を適用すると置き換えられます。
settings-builtin-glyphs-label = 組み込みグリフ
settings-builtin-glyphs-description = 有効にすると、Unicode の罫線・ブロック文字をフォントではなく専用の図形で描画し、すき間なくセルに接続します。変更はすぐに反映されます。
settings-builtin-glyphs-stored-invalid = settings.toml の値({ $value })は true / false ではないため、有効として扱われます。設定を変更すると置き換えられます。
settings-category-dopamine-title = ドーパミンモード
settings-category-dopamine-description = タイピングや操作に応じたダイナミックな視覚エフェクトと効果音です。
settings-dopamine-enabled-label = ドーパミンモードを有効にする
settings-dopamine-enabled-description = 有効にすると、ターミナルでの入力時に効果音とパーティクルエフェクトが発生します。変更はすぐに反映されます。
settings-dopamine-enabled-stored-invalid = settings.toml の値({ $value })は true / false ではないため、無効として扱われます。設定を変更すると置き換えられます。
settings-dopamine-sound-label = 効果音プロファイル
settings-dopamine-sound-zap = 電撃 (Zap)
settings-dopamine-sound-crack = 金属破砕 (Crack)
settings-dopamine-sound-burst = ネオン破裂 (Burst)
settings-dopamine-sound-preview = 試聴
settings-shell-mode-label = シェル
settings-shell-mode-description = ターミナルセッションで起動するシェルを選択します。
settings-shell-mode-auto = 自動検出
settings-shell-mode-custom = 実行ファイルを指定
settings-shell-path-label = 実行ファイルのパス
settings-shell-path-placeholder = C:\Program Files\PowerShell\7\pwsh.exe
settings-shell-integration-label = Shell Integration
settings-shell-integration-description = 自動の場合、対応シェル(PowerShell)でコマンド実行状態を検出します。無効の場合、Shell Integration を行いません。
settings-shell-integration-auto = 自動
settings-shell-integration-disabled = 無効
settings-shell-new-tab-note = 設定の変更は新しく開いたターミナルタブから反映されます。
settings-shell-path-empty = パスが入力されていません。
settings-shell-path-not-absolute = 絶対パスで指定してください (例: C:\...)。
settings-shell-path-not-found = 指定されたファイルが存在しません。
settings-shell-path-is-directory = 実行ファイルではなくディレクトリが指定されています。
settings-shell-path-not-executable = 有効な実行ファイル (.exe) ではありません。
settings-shell-path-has-arguments = 起動引数は含めず、実行ファイルのパス単体を指定してください。
settings-tab-close-tooltip = 設定を閉じる

## Main area

main-placeholder-title = ターミナル(プレースホルダー)
main-placeholder-body = ターミナルセッションはまだ実装されていません。

## Confirmation Dialog

dialog-smoke-question-title = スモークテスト質問
dialog-smoke-question-message = このテスト操作を実行しますか？
dialog-smoke-warning-title = スモークテスト警告
dialog-smoke-warning-message = 警告: このテスト操作は模擬的な破壊的アクションを実行します。

dialog-action-cancel = キャンセル
dialog-action-continue = 続行
dialog-action-ok = OK
dialog-action-close = 閉じる
dialog-action-delete = 削除
dialog-action-exit = 終了

## Font Picker

font-picker-title-ui = UI フォント
font-picker-title-terminal = ターミナル フォント
font-picker-search-placeholder = フォントを検索...
font-picker-monospace-only = 等幅フォントのみ
font-picker-preview-header = プレビュー
font-picker-no-matching-fonts = 該当するフォントがありません
font-picker-cancel = キャンセル
font-picker-select = 選択


