-- Nordlys on Hyprland (0.55 or newer, Lua config).
--
-- nordlys-session starts Hyprland with this file, so ~/.config/hypr is
-- left alone. To change something, put Lua in
-- ~/.config/nordlys/hyprland.lua: it runs last and can override any of this.
--
-- Hyprland has no Stage layout, so the session uses its monocle layout:
-- the focused window fills the stage and the others wait, unseen, on the
-- shelf. Super+T switches to a tiled view (master layout) to see windows
-- side by side. With nordlys-comp, the keys go to the real Stage model.

local data = os.getenv("NORDLYS_DATA") or "/usr/share/nordlys"
local home = os.getenv("HOME") or ""

-- Programs. Each tries the usual suspects in order.
local function first_of(list)
    return "sh -c 'for p in " .. list .. "; do command -v \"$p\" >/dev/null && exec \"$p\"; done'"
end
local terminal = first_of('"$TERMINAL" foot kitty alacritty ghostty wezterm konsole gnome-terminal')
local editor   = first_of('"$NORDLYS_EDITOR" gnome-text-editor kate code zeditor codium mousepad')
local files    = first_of('"$NORDLYS_FILES" nautilus thunar nemo dolphin pcmanfm-qt')
local browser  = first_of('"$BROWSER" firefox chromium zen-browser librewolf brave google-chrome-stable')
local monitor  = "sh -c 'command -v btop >/dev/null && exec foot -a btop -T \"System monitor\" btop; exec gnome-system-monitor'"

-- Screenshots: Super+Shift+S copies a region, Print saves the whole screen
-- to ~/Pictures/Screenshots (and copies it), Shift+Print saves a region.
local shots = "\"$(xdg-user-dir PICTURES 2>/dev/null || echo \"$HOME/Pictures\")/Screenshots\""
local function screenshot(region)
    local grab = region and 'grim -g "$(slurp)"' or "grim"
    return "sh -c 'd=" .. shots .. "; mkdir -p \"$d\"; f=\"$d/$(date +%Y-%m-%d_%H-%M-%S).png\"; "
        .. grab .. " \"$f\" && wl-copy <\"$f\" && notify-send -a Screenshot -i \"$f\" \"Screenshot saved\" \"$f\"'"
end

------------------
---- MONITORS ----
------------------

hl.monitor({ output = "", mode = "preferred", position = "auto", scale = "auto" })

-------------------
---- AUTOSTART ----
-------------------

-- Without the systemd user units (a build tree, or no systemd), start the
-- same pieces directly. They won't come back if they crash.
local idle = os.getenv("NORDLYS_IDLE") or (data .. "/hypridle.conf")
-- The wallpaper and its fit picked in Settings, else the Nordlys one.
local conf = (os.getenv("XDG_CONFIG_HOME") or (home .. "/.config")) .. "/nordlys"
local wallpaper, wallpaper_mode = data .. "/wallpaper.jpg", "fill"
local wf = io.open(conf .. "/wallpaper", "r")
if wf then
    wf:close()
    wallpaper = conf .. "/wallpaper"
end
local mf = io.open(conf .. "/wallpaper-mode", "r")
if mf then
    local m = mf:read("l")
    mf:close()
    if m == "fill" or m == "fit" or m == "center" or m == "tile" then wallpaper_mode = m end
end
local by_hand = table.concat({
    "nordlys-shell notify &",
    "nordlys-shell &",
    "nordlys-shell shelf &",
    "nordlys-shell ambient &",
    "nordlys-shell close-button &",
    "swaybg -m " .. wallpaper_mode .. " -i '" .. wallpaper .. "' &",
    "hypridle -c '" .. idle .. "' &",
    "{ command -v cliphist >/dev/null && wl-paste --watch cliphist store; } &",
}, " ")

hl.on("hyprland.start", function()
    -- Portals, D-Bus apps and the session's user services need the
    -- session's environment, so import it before starting them. The
    -- services restart if they crash (session/systemd/).
    hl.exec_cmd("sh -c 'dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP HYPRLAND_INSTANCE_SIGNATURE NORDLYS_DATA NORDLYS_IDLE PATH XDG_CONFIG_DIRS QT_QPA_PLATFORMTHEME SSH_AUTH_SOCK"
        .. " && systemctl --user --no-block start nordlys-session.target"
        .. " || { " .. by_hand:gsub("'", "'\\''") .. " }'")

    -- Theme GTK, Qt and the default apps from nordlys.toml, and write the
    -- starting dotfiles that are missing. Files you've made your own are
    -- left alone (see nordlys-theme --dry-run).
    hl.exec_cmd("nordlys-theme --quiet")
    -- ~/Pictures, ~/Downloads… for the file manager's sidebar and Print.
    hl.exec_cmd("sh -c 'command -v xdg-user-dirs-update >/dev/null && xdg-user-dirs-update'")

    -- In a test VM (scripts/setup.sh installs it): shared clipboard and
    -- a screen that follows the window size.
    hl.exec_cmd("sh -c 'command -v nordlys-vm-agent >/dev/null && exec nordlys-vm-agent'")

    -- Password prompts for apps that ask for admin rights.
    hl.exec_cmd("sh -c 'systemctl --user start hyprpolkitagent || /usr/lib/hyprpolkitagent/hyprpolkitagent'")

    -- The first login: what the Stage is and the keys (Super+F1 again later).
    hl.exec_cmd("sh -c 'sleep 2; exec nordlys-shell welcome --first-run'")
end)

-------------------------------
---- ENVIRONMENT VARIABLES ----
-------------------------------

hl.env("XCURSOR_SIZE", "24")
hl.env("HYPRCURSOR_SIZE", "24")
hl.env("QT_QPA_PLATFORMTHEME", "qt6ct")
hl.env("QT_QPA_PLATFORM", "wayland;xcb")
hl.env("GDK_BACKEND", "wayland,x11")
hl.env("MOZ_ENABLE_WAYLAND", "1")
hl.env("ELECTRON_OZONE_PLATFORM_HINT", "auto")

-----------------------
---- LOOK AND FEEL ----
-----------------------

-- The Nordlys palette: accent #8AB4FF, aurora violet #6D3FD9.
hl.config({
    general = {
        gaps_in     = 6,
        gaps_out    = 12,
        border_size = 1,
        col = {
            active_border   = { colors = { "rgba(8ab4ffcc)", "rgba(6d3fd9cc)" }, angle = 45 },
            inactive_border = "rgba(ffffff1f)",
        },
        resize_on_border = true,
        layout = "monocle",
    },

    decoration = {
        rounding       = 14,
        rounding_power = 2,
        active_opacity   = 1.0,
        inactive_opacity = 1.0,

        shadow = {
            enabled      = true,
            range        = 28,
            render_power = 3,
            color        = 0x8c000000,
        },

        -- Frosted glass for the shell's surfaces and any translucent window.
        blur = {
            enabled   = true,
            size      = 10,
            passes    = 3,
            noise     = 0.02,
            contrast  = 0.9,
            brightness = 0.85,
            vibrancy  = 0.17,
            popups    = true,
        },
    },

    animations = { enabled = true },

    master = {
        new_status = "master",   -- the newest window takes the stage
        new_on_top = true,       -- and the one it replaces leads the shelf
        mfact      = 0.68,
        orientation = "left",
    },

    misc = {
        force_default_wallpaper = 0,
        -- XDG_CURRENT_DESKTOP is set by nordlys-session, deliberately.
        disable_xdg_env_checks = true,
        disable_hyprland_logo   = true,
        disable_splash_rendering = true,
        -- If the lock screen crashes, let a new one take over.
        allow_session_lock_restore = true,
    },

    input = {
        kb_layout    = "us",
        follow_mouse = 1,
        touchpad = { natural_scroll = true, tap_to_click = true },
    },
})

hl.curve("nordlys", { type = "bezier", points = { {0.22, 1}, {0.36, 1} } })
hl.curve("easy",    { type = "spring", mass = 1, stiffness = 238.1191, damping = 24.21279333 })
hl.animation({ leaf = "global",     enabled = true, speed = 8,   bezier = "nordlys" })
hl.animation({ leaf = "windows",    enabled = true, speed = 4.8, spring = "easy" })
hl.animation({ leaf = "windowsIn",  enabled = true, speed = 4.1, spring = "easy", style = "popin 90%" })
hl.animation({ leaf = "windowsOut", enabled = true, speed = 1.5, bezier = "nordlys", style = "popin 90%" })
hl.animation({ leaf = "layersIn",   enabled = true, speed = 3,   bezier = "nordlys", style = "fade" })
hl.animation({ leaf = "layersOut",  enabled = true, speed = 1.5, bezier = "nordlys", style = "fade" })
hl.animation({ leaf = "workspaces", enabled = true, speed = 2.5, bezier = "nordlys", style = "slidefade 12%" })

hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })

-----------------------
---- SHELL SURFACES ---
-----------------------

-- Blur behind the bar, launcher, rails, toasts and the OSD. ignore_alpha keeps the
-- blur to the painted parts, not the clear margins around them.
hl.layer_rule({
    name  = "nordlys-glass",
    match = { namespace = "^nordlys-(bar|launcher|shelf|ambient|notify|osd|welcome)$" },
    blur  = true,
    ignore_alpha = 0.3,
})

-- Small utility windows float over the stage instead of taking it.
for i, class in ipairs({
    "^(org.pulseaudio.pavucontrol|pavucontrol)$",
    "^(nm-connection-editor|blueman-manager|.blueman-manager-wrapped)$",
    "^(xdg-desktop-portal-gtk|org.freedesktop.impl.portal.desktop.kde)$",
    "^(org.gnome.FileRoller|file-roller)$",
    "^(org.gnome.Calculator|qalculate-gtk)$",
    "^(hyprpolkitagent|polkit-gnome-authentication-agent-1)$",
    -- The keyring asking for its password, and printer setup.
    "^(gcr-prompter|org.gnome.keyring.SystemPrompter)$",
    "^(system-config-printer|org.gnome.Seahorse)$",
}) do
    hl.window_rule({ name = "nordlys-float-" .. i, match = { class = class }, float = true })
end
hl.window_rule({ name = "nordlys-float-pip", match = { title = "^(Picture-in-Picture|Picture in picture)$" }, float = true })
hl.window_rule({ name = "nordlys-float-dialogs", match = { title = "^(Open File|Save File|Open Folder|Save As|File Upload).*" }, float = true })

hl.window_rule({
    name  = "suppress-maximize-events",
    match = { class = ".*" },
    suppress_event = "maximize",
})

---------------------
---- KEYBINDINGS ----
---------------------

local super = "SUPER"
local function key(k) return super .. " + " .. k end
local function run(cmd) return hl.dsp.exec_cmd(cmd) end

-- Settings: when Hyprland reads this file straight from a checkout, that
-- checkout's own build comes first (target/ isn't on PATH), then the
-- installed one. Each path is one word inside first_of's 'sh -c' quotes.
local function sh_word(s)
    local w = s:gsub('[\\"$`]', "\\%0")
    w = w:gsub("'", "'\\''")
    return '"' .. w .. '"'
end
local settings = "nordlys-settings"
local here = debug and debug.getinfo and debug.getinfo(1, "S")
local repo = here and here.source:match("^@(.+)/session/[^/]+$")
if repo then
    for _, build in ipairs({ "release", "debug" }) do
        local bin = repo .. "/target/" .. build .. "/nordlys-settings"
        local f = io.open(bin, "r")
        if f then
            f:close()
            settings = sh_word(bin) .. " " .. settings
        end
    end
end

-- Is nordlys-comp running? (Its socket is there.)
local function comp_running()
    local sock = io.open((os.getenv("XDG_RUNTIME_DIR") or "/tmp") .. "/nordlys.sock", "r")
    if sock then sock:close() end
    return sock ~= nil
end

-- Stage. Monocle shows only the focused window, so bringing a window to
-- the stage is just focusing it. Super+T flips to the tiled view and back.
local tiled = false
local function when_tiled(a, b)
    return function() hl.dispatch(tiled and a or b) end
end
local last = hl.dsp.focus({ last = true })

-- Global on purpose: the launcher runs these with `hyprctl eval`.
nordlys_stage = {
    previous = function() hl.dispatch(last) end,
    cycle = when_tiled(hl.dsp.layout("rollnext"), hl.dsp.layout("cyclenext")),
    two_up = function()
        tiled = not tiled
        hl.config({ general = { layout = tiled and "master" or "monocle" } })
    end,
    swap = when_tiled(hl.dsp.layout("swapwithmaster"), last),
    -- On the stage view, shelving the window means bringing the previous one up.
    send_to_shelf = when_tiled(hl.dsp.layout("swapwithmaster"), last),
}

local function bind_stage(k, ctl, action, desc)
    hl.bind(key(k), function()
        if comp_running() then
            hl.exec_cmd("nordlysctl " .. ctl)
        else
            action()
        end
    end, { desc = desc })
end

bind_stage("K",   "previous",      nordlys_stage.previous,      "Previous window to stage")
bind_stage("J",   "cycle",         nordlys_stage.cycle,         "Cycle through the shelf")
bind_stage("T",   "two-up",        nordlys_stage.two_up,        "Two-up (tiled view) on or off")
bind_stage("Tab", "swap",          nordlys_stage.swap,          "Swap main and side")
bind_stage("S",   "send-to-shelf", nordlys_stage.send_to_shelf, "Send to shelf")

-- Shelves and groups
hl.bind(key("G"), run("nordlys-shell launcher --query '>'"), { desc = "App groups" })
hl.bind(key("Q"), hl.dsp.window.close(),                      { desc = "Close window" })

-- Workspaces
for i = 1, 9 do
    hl.bind(key(i),              hl.dsp.focus({ workspace = i }))
    hl.bind(key("SHIFT + " .. i), hl.dsp.window.move({ workspace = i }))
end
hl.bind(key("bracketleft"),  hl.dsp.focus({ workspace = "e-1" }), { desc = "Previous workspace" })
hl.bind(key("bracketright"), hl.dsp.focus({ workspace = "e+1" }), { desc = "Next workspace" })
hl.bind(key("mouse_down"),   hl.dsp.focus({ workspace = "e+1" }))
hl.bind(key("mouse_up"),     hl.dsp.focus({ workspace = "e-1" }))
hl.bind(key("minus"),         hl.dsp.workspace.toggle_special("scratch"), { desc = "Scratchpad" })
hl.bind(key("SHIFT + minus"), hl.dsp.window.move({ workspace = "special:scratch" }))
hl.bind(key("F"),             hl.dsp.window.float({ action = "toggle" }))

-- Launch and system
hl.bind(key("SPACE"),         run("nordlys-shell launcher"),               { desc = "Launcher" })
hl.bind(key("SHIFT + SPACE"), run("nordlys-shell launcher --query '>'"),   { desc = "Commands" })
hl.bind(key("RETURN"),        run(terminal),                               { desc = "Terminal" })
hl.bind(key("E"),             run(editor),                                 { desc = "Editor" })
hl.bind(key("N"),             run(files),                                  { desc = "Files" })
hl.bind(key("B"),             run(browser),                                { desc = "Browser" })
hl.bind("CTRL + SHIFT + ESCAPE", run(monitor),                             { desc = "System monitor" })
hl.bind(key("comma"),         run(first_of(settings) .. " || xdg-open '" .. conf .. "/nordlys.toml'"), { desc = "Settings" })
hl.bind(key("SHIFT + S"),     run("sh -c 'grim -g \"$(slurp)\" - | wl-copy'"), { desc = "Screenshot to clipboard" })
-- Screen recording to ~/Videos/Recordings; the same key stops it.
hl.bind(key("R"),             run("nordlys-record region"),                { desc = "Record a region" })
hl.bind(key("SHIFT + R"),     run("nordlys-record screen"),                { desc = "Record the screen" })
hl.bind(key("V"),             run("nordlys-shell launcher --query '# '"),  { desc = "Clipboard history" })
hl.bind(key("F1"),            run("nordlys-shell welcome"),                { desc = "Welcome and keys" })
hl.bind("Print",              run(screenshot(false)),                      { desc = "Save a screenshot" })
hl.bind("SHIFT + Print",      run(screenshot(true)),                       { desc = "Save a screenshot of a region" })
hl.bind(key("ESCAPE"),        run("nordlys-shell lock"),                   { desc = "Lock" })
hl.bind(key("L"),             run("nordlys-shell lock"),                   { desc = "Lock" })
-- Logging out closes every app, so it takes the keys twice within three
-- seconds. The first press says so in a notification.
local logout_asked = -math.huge
hl.bind(key("SHIFT + ESCAPE"), function()
    local now = os.time()
    if now - logout_asked <= 3 then
        hl.dispatch(hl.dsp.exit())
    else
        logout_asked = now
        hl.exec_cmd("notify-send -a Nordlys -u critical -t 3000 -h string:x-canonical-private-synchronous:nordlys-logout "
            .. "'Log out?' 'Press Super+Shift+Esc again to log out.'")
    end
end, { desc = "Log out (press twice)" })

-- Mouse: Super+drag moves, Super+right-drag resizes.
hl.bind(key("mouse:272"), hl.dsp.window.drag(),   { mouse = true })
hl.bind(key("mouse:273"), hl.dsp.window.resize(), { mouse = true })

-- Media and brightness keys, also on the lock screen. `nordlys-shell osd`
-- makes the change and shows the new level in a pill near the bottom.
local media = { locked = true, repeating = true }
hl.bind("XF86AudioRaiseVolume",  run("nordlys-shell osd volume up"),       media)
hl.bind("XF86AudioLowerVolume",  run("nordlys-shell osd volume down"),     media)
hl.bind("XF86AudioMute",         run("nordlys-shell osd volume mute"),     media)
hl.bind("XF86AudioMicMute",      run("nordlys-shell osd mic mute"),        media)
hl.bind("XF86MonBrightnessUp",   run("nordlys-shell osd brightness up"),   media)
hl.bind("XF86MonBrightnessDown", run("nordlys-shell osd brightness down"), media)
hl.bind("XF86AudioNext",  run("playerctl next"),       { locked = true })
hl.bind("XF86AudioPrev",  run("playerctl previous"),   { locked = true })
hl.bind("XF86AudioPlay",  run("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioPause", run("playerctl play-pause"), { locked = true })

-- Test VMs: virtio-gpu draws the cursor wrongly on a hardware plane.
local vm = io.open(data .. "/vm-guest", "r")
if vm then
    vm:close()
    hl.config({ cursor = { no_hardware_cursors = true } })
end

-- What you changed in Settings (nordlys-settings writes this file). A
-- broken one is skipped, so the session still starts.
local made = conf .. "/settings.lua"
local sf = io.open(made, "r")
if sf then
    sf:close()
    local ok, err = pcall(dofile, made)
    if not ok then
        hl.exec_cmd("notify-send -a Nordlys -u critical 'Settings not applied' '" .. tostring(err):gsub("'", "") .. "'")
    end
end

-----------------------
---- YOUR CHANGES -----
-----------------------

local user = home .. "/.config/nordlys/hyprland.lua"
local f = io.open(user, "r")
if f then
    f:close()
    dofile(user)
end
