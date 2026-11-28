-- Tests for session/hyprland.lua with a stand-in for Hyprland's `hl` API.
--
--   lua hyprland_test.lua path/to/hyprland.lua           run the tests
--   lua hyprland_test.lua path/to/hyprland.lua commands  print the commands
--       the config would run, NUL-separated, for test_hyprland.py
--
-- Nothing is executed: exec_cmd only records, and the environment, the
-- files the config looks for and the clock are all made up.

local config_path, mode = arg[1], arg[2]
assert(config_path, "usage: hyprland_test.lua hyprland.lua [commands]")

local real_open, real_dofile = io.open, dofile
local real_getenv, real_time = os.getenv, os.time

--------------------------------------------------------------------------
-- The stand-in

-- hl.dsp.window.move(x) and friends become { path = "window.move", args }.
local function dispatcher(path)
    return setmetatable({}, {
        __index = function(_, name)
            return dispatcher(path == "" and name or path .. "." .. name)
        end,
        __call = function(_, ...)
            return { path = path, args = { ... } }
        end,
    })
end

-- Load the config in a fresh world.
--   env: environment variables
--   files: paths that exist (the value is what reading one returns)
--   dofile: what running another config does
local function load(world)
    world = world or {}
    local env = world.env or { HOME = "/home/u", XDG_RUNTIME_DIR = "/run/user/1" }
    local files = world.files or {}
    local seen = {
        monitors = {}, execs = {}, env = {}, configs = {}, curves = {},
        animations = {}, gestures = {}, layer_rules = {}, window_rules = {},
        binds = {}, order = {}, dispatched = {}, ran = {}, handlers = {},
        now = 1000,
    }

    local function into(list)
        return function(value) list[#list + 1] = value end
    end

    local hl = {
        monitor = into(seen.monitors),
        exec_cmd = into(seen.execs),
        config = into(seen.configs),
        animation = into(seen.animations),
        gesture = into(seen.gestures),
        layer_rule = into(seen.layer_rules),
        window_rule = into(seen.window_rules),
        dispatch = into(seen.dispatched),
        env = function(k, v) seen.env[k] = v end,
        curve = function(name, spec) seen.curves[name] = spec end,
        on = function(event, f) seen.handlers[event] = f end,
        bind = function(keys, action, options)
            assert(seen.binds[keys] == nil, "bound twice: " .. keys)
            assert(action ~= nil, "no action: " .. keys)
            seen.binds[keys] = { action = action, options = options or {} }
            seen.order[#seen.order + 1] = keys
        end,
        dsp = dispatcher(""),
    }

    local sandbox_os = {
        getenv = function(name) return env[name] end,
        time = function() return seen.now end,
    }
    local sandbox_io = {
        open = function(path)
            local found = files[path]
            if not found then return nil end
            return {
                close = function() end,
                read = function()
                    if found == true or found == "" then return nil end
                    return (found:match("[^\n]*"))
                end,
            }
        end,
    }
    local globals = setmetatable({
        hl = hl, os = sandbox_os, io = sandbox_io,
        dofile = function(path)
            seen.ran[#seen.ran + 1] = path
            if world.dofile then return world.dofile(path) end
        end,
    }, { __index = _G })

    local chunk = assert(loadfile(config_path, "t", globals))
    chunk()
    seen.globals = globals
    return seen
end

--------------------------------------------------------------------------
-- For test_hyprland.py: the shell commands, to run with fake programs.

if mode == "commands" then
    local w = load({ env = {
        HOME = "/home/u",
        NORDLYS_DATA = real_getenv("NORDLYS_TEST_DATA"),
        NORDLYS_IDLE = real_getenv("NORDLYS_TEST_IDLE"),
    } })
    w.handlers["hyprland.start"]()
    local function emit(name, command)
        io.write(name, "\0", command, "\0")
    end
    for i, command in ipairs(w.execs) do
        emit("start" .. i, command)
    end
    for _, keys in ipairs(w.order) do
        local action = w.binds[keys].action
        if type(action) == "table" and action.path == "exec_cmd" then
            emit(keys, action.args[1])
        end
    end
    os.exit(0)
end

--------------------------------------------------------------------------
-- Tests

local tests, names = {}, {}
local function test(name, f)
    tests[name] = f
    names[#names + 1] = name
end

local function same(got, want, what)
    if got ~= want then
        error(string.format("%s: got %s, want %s", what or "value",
            tostring(got), tostring(want)), 2)
    end
end

local function has(text, part)
    if not tostring(text):find(part, 1, true) then
        error(string.format("%q is not in %q", part, tostring(text)), 2)
    end
end

local function press(w, keys)
    local bind = assert(w.binds[keys], "not bound: " .. keys)
    same(type(bind.action), "function", keys .. " action")
    bind.action()
end

local function command(w, keys)
    local action = assert(w.binds[keys], "not bound: " .. keys).action
    same(action.path, "exec_cmd", keys)
    return action.args[1]
end

local function layouts(w)
    local seen = {}
    for _, c in ipairs(w.configs) do
        if c.general and c.general.layout then
            seen[#seen + 1] = c.general.layout
        end
    end
    return seen
end

test("it sets up the monitor, environment and look", function()
    local w = load()
    same(#w.monitors, 1, "monitors")
    same(w.monitors[1].mode, "preferred")
    for name, value in pairs({
        XCURSOR_SIZE = "24", HYPRCURSOR_SIZE = "24",
        QT_QPA_PLATFORMTHEME = "qt6ct", QT_QPA_PLATFORM = "wayland;xcb",
        GDK_BACKEND = "wayland,x11", MOZ_ENABLE_WAYLAND = "1",
        ELECTRON_OZONE_PLATFORM_HINT = "auto",
    }) do
        same(w.env[name], value, name)
    end
    local main = w.configs[1]
    same(main.general.layout, "monocle")
    same(main.misc.allow_session_lock_restore, true,
        "a crashed lock screen can be replaced")
    same(main.misc.disable_xdg_env_checks, true)
end)

test("every curve an animation names is defined", function()
    local w = load()
    assert(#w.animations > 0)
    for _, a in ipairs(w.animations) do
        local curve = a.bezier or a.spring
        assert(w.curves[curve], "unknown curve " .. tostring(curve))
    end
end)

test("the shell's surfaces are blurred", function()
    local rule = load().layer_rules[1]
    same(rule.blur, true)
    for _, piece in ipairs({ "bar", "launcher", "shelf", "notify", "osd", "welcome" }) do
        has(rule.match.namespace, piece)
    end
end)

test("small utility windows float over the stage", function()
    local floating, names = {}, {}
    for _, rule in ipairs(load().window_rules) do
        assert(not names[rule.name], "two rules named " .. tostring(rule.name))
        names[rule.name] = true
        if rule.float == true then
            floating[#floating + 1] = tostring(rule.match.class or rule.match.title)
        end
    end
    local all = table.concat(floating, " ")
    for _, app in ipairs({ "pavucontrol", "blueman", "portal-gtk", "polkit", "gcr-prompter",
                           "Picture-in-Picture", "Save File" }) do
        has(all, app)
    end
end)

test("nothing runs until Hyprland starts", function()
    local w = load()
    same(#w.execs, 0)
    w.handlers["hyprland.start"]()
    same(#w.execs, 6)
    has(w.execs[1], "systemctl --user --no-block start nordlys-session.target")
    same(w.execs[2], "nordlys-theme --quiet --watch")
    has(w.execs[3], "xdg-user-dirs-update")
    has(w.execs[4], "nordlys-vm-agent")
    has(w.execs[5], "hyprpolkitagent")
    has(w.execs[6], "nordlys-shell welcome --first-run")
end)

test("data and idle settings come from the session script", function()
    local w = load({ env = { HOME = "/h", NORDLYS_DATA = "/d", NORDLYS_IDLE = "/i.conf" } })
    w.handlers["hyprland.start"]()
    has(w.execs[1], "/d/wallpaper.jpg")
    has(w.execs[1], "/i.conf")
    local plain = load({ env = {} })
    plain.handlers["hyprland.start"]()
    has(plain.execs[1], "/usr/share/nordlys/wallpaper.jpg")
    has(plain.execs[1], "/usr/share/nordlys/hypridle.conf")
    has(plain.execs[1], "swaybg -m fill ")
end)

test("the wallpaper and fit picked in Settings are used", function()
    local function start(files, env)
        local w = load({ env = env or { HOME = "/h", NORDLYS_DATA = "/d" }, files = files })
        w.handlers["hyprland.start"]()
        return w.execs[1]
    end
    local mine = start({ ["/h/.config/nordlys/wallpaper"] = true })
    has(mine, "/h/.config/nordlys/wallpaper")
    assert(not mine:find("/d/wallpaper.jpg", 1, true))
    for _, fit in ipairs({ "fill", "fit", "center", "tile" }) do
        has(start({ ["/h/.config/nordlys/wallpaper-mode"] = fit .. "\n" }), "swaybg -m " .. fit .. " ")
    end
    for _, junk in ipairs({ "stretch; rm -rf /", "" }) do
        has(start({ ["/h/.config/nordlys/wallpaper-mode"] = junk }), "swaybg -m fill ")
    end
    local xdg = start({ ["/x/nordlys/wallpaper"] = true }, { HOME = "/h", XDG_CONFIG_HOME = "/x" })
    has(xdg, "/x/nordlys/wallpaper")
end)

test("what Settings wrote is applied", function()
    local made = "/h/.config/nordlys/settings.lua"
    local w = load({ env = { HOME = "/h" }, files = { [made] = true } })
    same(w.ran[1], made)
    same(#w.execs, 0)
end)

test("a broken settings file doesn't stop the session", function()
    local made = "/h/.config/nordlys/settings.lua"
    local w = load({
        env = { HOME = "/h" },
        files = { [made] = true },
        dofile = function() error("it's broken") end,
    })
    same(#w.execs, 1)
    has(w.execs[1], "Settings not applied")
    has(w.execs[1], "its broken")
    assert(w.binds["SUPER + SPACE"], "the rest of the config still loads")
end)

test("programs fall back down a list, yours first", function()
    local w = load()
    has(command(w, "SUPER + RETURN"), '"$TERMINAL" foot kitty')
    has(command(w, "SUPER + E"), '"$NORDLYS_EDITOR" gnome-text-editor')
    has(command(w, "SUPER + N"), '"$NORDLYS_FILES" nautilus')
    has(command(w, "SUPER + B"), '"$BROWSER" firefox')
    has(command(w, "SUPER + comma"), "nordlys-settings")
    has(command(w, "SUPER + comma"), "|| xdg-open")
end)

local STAGE = {
    { key = "SUPER + K",   ctl = "previous" },
    { key = "SUPER + J",   ctl = "cycle" },
    { key = "SUPER + T",   ctl = "two-up" },
    { key = "SUPER + Tab", ctl = "swap" },
    { key = "SUPER + S",   ctl = "send-to-shelf" },
}

test("with nordlys-comp running, stage keys go to it", function()
    local w = load({ env = { XDG_RUNTIME_DIR = "/run/u" },
                     files = { ["/run/u/nordlys.sock"] = true } })
    for i, s in ipairs(STAGE) do
        press(w, s.key)
        same(w.execs[i], "nordlysctl " .. s.ctl, s.key)
    end
    same(#w.dispatched, 0)
    same(#layouts(w), 1, "the layout is left alone")
end)

test("without a runtime dir the socket is looked for in /tmp", function()
    local w = load({ env = {}, files = { ["/tmp/nordlys.sock"] = true } })
    press(w, "SUPER + J")
    same(w.execs[1], "nordlysctl cycle")
end)

test("on the stage view the keys use monocle focus", function()
    local w = load()
    for _, s in ipairs(STAGE) do
        if s.ctl ~= "two-up" then press(w, s.key) end
    end
    same(#w.execs, 0)
    same(w.dispatched[1].path, "focus")
    same(w.dispatched[1].args[1].last, true, "previous")
    same(w.dispatched[2].args[1], "cyclenext", "cycle")
    same(w.dispatched[3].args[1].last, true, "swap")
    same(w.dispatched[4].args[1].last, true, "send to shelf")
end)

test("two-up switches to a tiled view and back", function()
    local w = load()
    press(w, "SUPER + T")
    press(w, "SUPER + J")
    press(w, "SUPER + Tab")
    press(w, "SUPER + S")
    same(w.dispatched[1].args[1], "rollnext")
    same(w.dispatched[2].args[1], "swapwithmaster")
    same(w.dispatched[3].args[1], "swapwithmaster")
    press(w, "SUPER + T")
    press(w, "SUPER + J")
    same(w.dispatched[4].args[1], "cyclenext")
    local seen = layouts(w)
    same(table.concat(seen, " "), "monocle master monocle")
end)

test("the launcher can call the stage functions by name", function()
    local stage = load().globals.nordlys_stage
    for _, name in ipairs({ "previous", "cycle", "two_up", "swap", "send_to_shelf" }) do
        same(type(stage[name]), "function", name)
    end
end)

test("nine workspaces can be visited and filled", function()
    local w = load()
    for i = 1, 9 do
        local go = w.binds["SUPER + " .. i].action
        same(go.path, "focus")
        same(go.args[1].workspace, i)
        local move = w.binds["SUPER + SHIFT + " .. i].action
        same(move.path, "window.move")
        same(move.args[1].workspace, i)
    end
end)

test("logging out takes two presses within three seconds", function()
    local w = load()
    local keys = "SUPER + SHIFT + ESCAPE"
    press(w, keys)
    same(#w.dispatched, 0, "the first press only asks")
    has(w.execs[1], "notify-send")
    has(w.execs[1], "again to log out")
    w.now = w.now + 4
    press(w, keys)
    same(#w.dispatched, 0, "too slow: asks again")
    same(#w.execs, 2)
    w.now = w.now + 3
    press(w, keys)
    same(w.dispatched[1].path, "exit")
end)

test("media keys work on the lock screen", function()
    local w = load()
    local count = 0
    for keys, bind in pairs(w.binds) do
        if keys:find("^XF86") then
            count = count + 1
            same(bind.options.locked, true, keys)
        end
    end
    same(count, 10)
end)

test("dragging uses the mouse binds", function()
    local w = load()
    same(w.binds["SUPER + mouse:272"].options.mouse, true)
    same(w.binds["SUPER + mouse:273"].options.mouse, true)
end)

test("a test VM gets a software cursor", function()
    local function software(w)
        for _, c in ipairs(w.configs) do
            if c.cursor then return c.cursor.no_hardware_cursors end
        end
        return false
    end
    same(software(load()), false)
    same(software(load({ env = { NORDLYS_DATA = "/d" },
                         files = { ["/d/vm-guest"] = true } })), true)
end)

test("your own config runs last, when there is one", function()
    same(#load().ran, 0)
    local mine = "/home/u/.config/nordlys/hyprland.lua"
    local made = "/home/u/.config/nordlys/settings.lua"
    local w = load({ env = { HOME = "/home/u" }, files = { [mine] = true, [made] = true } })
    same(table.concat(w.ran, " "), made .. " " .. mine, "yours wins over Settings")
end)

local failed = 0
for _, name in ipairs(names) do
    local ok, problem = pcall(tests[name])
    if not ok then
        failed = failed + 1
        print("FAIL " .. name .. "\n  " .. tostring(problem))
    end
end
print(string.format("%d tests, %d failed", #names, failed))
io.open, dofile, os.getenv, os.time = real_open, real_dofile, real_getenv, real_time
os.exit(failed == 0 and 0 or 1)
