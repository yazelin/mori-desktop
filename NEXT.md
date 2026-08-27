# Next

- [ ] ydotool 語法未防呆:selection_linux.rs 寫死 1.x keycode 語法(29:1 47:1 47:0 29:0)。ydotool 0.1.x(Ubuntu 24.04 內建)只吃名字語法 ctrl+v,收到 keycode 語法會 exit 0 但送出錯的鍵(實測 42:1 送出 keycode 5=數字鍵4),結果是焦點視窗被打進垃圾字元、log 卻回報貼回完成。26.04 是 1.x 所以現在沒事;哪天在 24.04 跑 mori-desktop 才會踩到。修法照抄 mori-ear 的 ydotool_wants_named_keys(跑一次 ydotool key --help,說明有 separated by plus 就是 0.1.x)。另:Wayland 下 capture_window_context 的 xdotool getactivewindow 必失敗,terminal 偵測失效一律送 Ctrl+V,退路是 voice profile frontmatter 寫 paste_shortcut: ctrl+shift+v
