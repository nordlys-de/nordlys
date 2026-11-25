-- Nordlys on Hyprland (0.55 or newer, Lua config).
--
-- nordlys-session starts Hyprland with this file, so ~/.config/hypr is
-- left alone. To change something, put Lua in
-- ~/.config/nordlys/hyprland.lua: it runs last and can override any of this.
--
-- Hyprland has no Stage layout yet, so the Stage keys are mapped onto its
-- master layout: the master window is on stage, the stack is the shelf.
-- With nordlys-comp, the same keys go to the real Stage model.

local data = os.getenv("NORDLYS_DATA") or "/usr/share/nordlys"
local home = os.getenv("HOME") or ""

-- Programs. Each tries the usual suspects in order.
local function first_of(list)
    return "sh -c 'for p in " .. list .. "; do command -v \"$p\" >/dev/null && exec \"$p\"; done'"
end
local terminal = first_of('"$TERMINAL" foot kitty alacritty ghostty wezterm konsole gnome-terminal')
local editor   = first_of('"$NORDLYS_EDITOR" gnome-text-editor kate code zeditor codium mousepad')

------------------
---- MONITORS ----
------------------

hl.monitor({ output = "", mode = "preferred", position = "auto", scale = "auto" })

-------------------
---- AUTOSTART ----
-------------------

hl.on("hyprland.start", function()
    -- Portals and apps started by D-Bus need the session's environment.
    hl.exec_cmd("dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE HYPRLAND_INSTANCE_SIGNATURE")

    hl.exec_cmd("swaybg -m fill -i " .. data .. "/wallpaper.jpg")
    -- Theme GTK and Qt apps on first login, unless you have your own gtk.css.
    hl.exec_cmd("sh -c 'c=${XDG_CONFIG_HOME:-$HOME/.config}; [ -e $c/gtk-4.0/gtk.css ] || [ -e $c/gtk-3.0/gtk.css ] || nordlys-theme'")
    hl.exec_cmd("nordlys-shell notify")
    hl.exec_cmd("nordlys-shell")
    hl.exec_cmd("nordlys-shell shelf")
    hl.exec_cmd("nordlys-shell ambient")
    hl.exec_cmd("hypridle -c " .. (os.getenv("NORDLYS_IDLE") or (data .. "/hypridle.conf")))

    -- Password prompts for apps that ask for admin rights.
    hl.exec_cmd("sh -c 'systemctl --user start hyprpolkitagent || /usr/lib/hyprpolkitagent/hyprpolkitagent'")
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
        layout = "master",
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

-- Blur behind the bar, launcher, rails and toasts. ignore_alpha keeps the
-- blur to the painted parts, not the clear margins around them.
hl.layer_rule({
    name  = "nordlys-glass",
    match = { namespace = "^nordlys-(bar|launcher|shelf|ambient|notify)$" },
    blur  = true,
    ignore_alpha = 0.3,
})

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

-- Is nordlys-comp running? (Its socket is there.)
local function comp_running()
    local sock = io.open((os.getenv("XDG_RUNTIME_DIR") or "/tmp") .. "/nordlys.sock", "r")
    if sock then sock:close() end
    return sock ~= nil
end

-- With nordlys-comp running, Stage keys go to it; on Hyprland, to master.
local function stage(ctl, ...)
    local fallback = { ... }
    return function()
        if comp_running() then
            hl.exec_cmd("nordlysctl " .. ctl)
            return
        end
        for _, d in ipairs(fallback) do hl.dispatch(d) end
    end
end

-- Stage
hl.bind(key("K"),   stage("previous",      hl.dsp.focus({ last = true }), hl.dsp.layout("swapwithmaster")),
        { desc = "Previous window to stage" })
hl.bind(key("J"),   stage("cycle",         hl.dsp.layout("rollnext")),        { desc = "Cycle through the shelf" })
local two_up = false
hl.bind(key("T"), function()
    if comp_running() then
        hl.exec_cmd("nordlysctl two-up")
        return
    end
    two_up = not two_up
    hl.dispatch(hl.dsp.layout(two_up and "addmaster" or "removemaster"))
end, { desc = "Two-up on or off" })
hl.bind(key("Tab"), stage("swap",          hl.dsp.layout("swapnext")),        { desc = "Swap main and side" })
hl.bind(key("S"),   stage("send-to-shelf", hl.dsp.layout("swapwithmaster")),  { desc = "Send to shelf" })

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
hl.bind(key("comma"),         run("xdg-open " .. home .. "/.config/nordlys/nordlys.toml"), { desc = "Settings" })
hl.bind(key("SHIFT + S"),     run("sh -c 'grim -g \"$(slurp)\" - | wl-copy'"), { desc = "Screenshot to clipboard" })
hl.bind(key("ESCAPE"),        run("nordlys-shell lock"),                   { desc = "Lock" })
hl.bind(key("L"),             run("nordlys-shell lock"),                   { desc = "Lock" })
hl.bind(key("SHIFT + ESCAPE"), hl.dsp.exit(),                              { desc = "Log out" })

-- Mouse: Super+drag moves, Super+right-drag resizes.
hl.bind(key("mouse:272"), hl.dsp.window.drag(),   { mouse = true })
hl.bind(key("mouse:273"), hl.dsp.window.resize(), { mouse = true })

-- Media and brightness keys, also on the lock screen.
local media = { locked = true, repeating = true }
hl.bind("XF86AudioRaiseVolume",  run("wpctl set-volume -l 1 @DEFAULT_AUDIO_SINK@ 5%+"), media)
hl.bind("XF86AudioLowerVolume",  run("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),      media)
hl.bind("XF86AudioMute",         run("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),     media)
hl.bind("XF86AudioMicMute",      run("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"),   media)
hl.bind("XF86MonBrightnessUp",   run("brightnessctl -e4 -n2 set 5%+"),                  media)
hl.bind("XF86MonBrightnessDown", run("brightnessctl -e4 -n2 set 5%-"),                  media)
hl.bind("XF86AudioNext",  run("playerctl next"),       { locked = true })
hl.bind("XF86AudioPrev",  run("playerctl previous"),   { locked = true })
hl.bind("XF86AudioPlay",  run("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioPause", run("playerctl play-pause"), { locked = true })

-----------------------
---- YOUR CHANGES -----
-----------------------

local user = home .. "/.config/nordlys/hyprland.lua"
local f = io.open(user, "r")
if f then
    f:close()
    dofile(user)
end
