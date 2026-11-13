//! The guide describes implemented input routes; it registers no hotkeys.
pub const GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Open and close", &[
        ("Touchpad: scroll right / left", "Over the closed island: Search / AI companion"),
        ("Mouse wheel: down / up", "Over the closed island: Search / AI, when enabled"),
        ("Escape or a fresh horizontal swipe", "Close Search or the island AI workspace"),
        ("Ctrl + hover over the island", "Reveal the AI shortcut, when AI is enabled"),
        ("Shift + mouse wheel", "Cycle the closed island's compact views"),
    ]),
    ("Search", &[
        ("Type a name", "Show matching apps, settings and enabled sources"),
        ("Up / Down or Tab", "Select a result or a recent application"),
        ("Page Up / Page Down", "Move the result selection by five rows"),
        ("Enter", "Open the selected result"),
        ("Ctrl + Enter", "Search the web; an empty field opens the browser"),
        ("Shift + F10 or Menu key", "Open the selected result's context menu"),
    ]),
    ("Browser", &[
        ("Ctrl + L", "Edit and select the full address"),
        ("Up / Down, then Enter", "Choose and open an address-bar suggestion"),
        ("Tab with suggestions open", "Fill the address field with the selected suggestion"),
        ("Alt + Left / Alt + Right", "Go back / forward in page history"),
        ("Ctrl + R or F5", "Reload the current webpage"),
        ("Escape", "Cancel address editing first; otherwise close the workspace"),
        ("Ctrl + A / C / X / V", "Select all / copy / cut / paste in the address field"),
    ]),
    ("AI companion: local provider chat", &[
        ("Enter / Shift + Enter", "Send a prompt / insert a new line"),
        ("Ctrl + N / Ctrl + Shift + N", "Start a new chat / temporary chat"),
        ("Ctrl + H", "Open local chat history"),
        ("Ctrl + K", "Open the model picker"),
        ("Ctrl + V", "Paste text or a supported attachment"),
        ("AI websites", "Their own shortcuts apply inside the webpage"),
    ]),
    ("Other controls", &[
        ("Alt + Shift + S", "Pause or resume Script reading, when available"),
        ("Alt + Shift + F", "Hide or show the game readout while a game is in front"),
        ("Alt + F4", "Close a detached browser or AI window"),
    ]),
];
