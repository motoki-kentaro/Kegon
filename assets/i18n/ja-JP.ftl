# Kegon UI strings: ja-JP.
#
# Messages missing here fall back to en-US at runtime, but the test suite
# expects this locale to be complete.

## Activity Bar (tooltips)

activity-explorer = エクスプローラー
activity-search = 検索
activity-git = Git
activity-settings = 設定

## Side Bar

side-bar-explorer-title = エクスプローラー
side-bar-explorer-placeholder = ファイルエクスプローラーはまだ実装されていません。
side-bar-search-title = 検索
side-bar-search-placeholder = プロジェクト全体の検索はまだ実装されていません。
side-bar-git-title = ソース管理
side-bar-git-placeholder = Git 連携はまだ実装されていません。
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
terminal-new-tab-tooltip = 新しいターミナル
terminal-close-tab-tooltip = ターミナルを閉じる
terminal-process-exited = プロセスがコード { $code } で終了しました。

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


