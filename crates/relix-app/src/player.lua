-- Relix player controls: a Netflix-style skin drawn with mpv's OSD (ASS).
-- Talks to the app through `script-message relix-next`; everything else is plain mpv commands.

local options = require "mp.options"
local opts = { title = "", episode = "", has_next = false }
options.read_options(opts, "relix")

local RED = "1409E5" -- #E50914 in ASS's BGR order
local HIDE_AFTER = 3 -- seconds without mouse movement
local NEXT_CARD_AT = 20 -- seconds before the end

local ov = mp.create_osd_overlay("ass-events")
local state = {
    w = 1280, h = 720, mx = -1, my = -1, last_move = 0,
    pause = false, pos = 0, dur = 0, volume = 100, mute = false, speed = 1, fullscreen = false,
    tracks = {}, panel = false, loaded = false,
}
local hits = {} -- clickable areas from the last render: {x1, y1, x2, y2, fn}

local function esc(s)
    return (tostring(s):gsub("\\", "\\\\"):gsub("{", "\\{"):gsub("}", "\\}"):gsub("\n", " "))
end

local function fmt_time(t)
    t = math.max(0, math.floor(t or 0))
    local h, m, s = math.floor(t / 3600), math.floor(t % 3600 / 60), t % 60
    if h > 0 then return string.format("%d:%02d:%02d", h, m, s) end
    return string.format("%d:%02d", m, s)
end

local function hovered(x1, y1, x2, y2)
    return state.mx >= x1 and state.mx <= x2 and state.my >= y1 and state.my <= y2
end

-- A filled rectangle; `alpha` is ASS alpha (00 opaque .. FF clear).
local function box(x, y, w, h, color, alpha)
    return string.format("{\\an7\\pos(%d,%d)\\bord0\\shad0\\1c&H%s&\\1a&H%s&\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}",
        x, y, color, alpha or "00", w, w, h, h)
end

-- A rectangle outline `t` thick.
local function outline(x, y, w, h, color, t)
    return string.format("{\\an7\\pos(%d,%d)\\bord%g\\shad0\\1a&HFF&\\3c&H%s&\\p1}m 0 0 l %d 0 l %d %d l 0 %d{\\p0}",
        x, y, t / 2, color or "FFFFFF", w, w, h, h)
end

-- A dark band with blurred edges: a gradient without visible steps.
local function fade(y, h, blur)
    return string.format("{\\an7\\pos(0,%d)\\bord0\\shad0\\blur%d\\1c&H000000&\\1a&H40&\\p1}m %d 0 l %d 0 l %d %d l %d %d{\\p0}",
        y, blur, -blur, state.w + blur, state.w + blur, h, -blur, h)
end

local function text(x, y, an, size, str, color, bold)
    return string.format("{\\an%d\\pos(%d,%d)\\fs%d\\bord0\\shad1\\4a&H80&\\1c&H%s&\\b%d}%s",
        an, x, y, size, color or "FFFFFF", bold and 1 or 0, esc(str))
end

-- Vector icons, drawn in a `size` square at (x, y).
local function icon(kind, x, y, size, color)
    local s = size
    local p
    if kind == "play" then
        p = string.format("m %d 0 l %d %d l %d %d", s * 0.15, s * 0.95, s / 2, s * 0.15, s)
    elseif kind == "pause" then
        p = string.format("m %d 0 l %d 0 l %d %d l %d %d m %d 0 l %d 0 l %d %d l %d %d",
            s * 0.15, s * 0.4, s * 0.4, s, s * 0.15, s, s * 0.6, s * 0.85, s * 0.85, s, s * 0.6, s)
    elseif kind == "next" then
        p = string.format("m 0 0 l %d %d l 0 %d m %d 0 l %d 0 l %d %d l %d %d",
            s * 0.7, s / 2, s, s * 0.75, s * 0.95, s * 0.95, s, s * 0.75, s)
    elseif kind == "speaker" then
        p = string.format("m 0 %d l %d %d l %d 0 l %d %d l %d %d l 0 %d",
            s * 0.3, s * 0.3, s * 0.3, s * 0.6, s * 0.6, s, s * 0.3, s * 0.7, s * 0.7)
    elseif kind == "fullscreen" then
        return outline(x + s * 0.1, y + s * 0.15, s * 0.8, s * 0.7, color, s * 0.08)
    end
    return string.format("{\\an7\\pos(%d,%d)\\bord0\\shad1\\4a&H80&\\1c&H%s&\\p1}%s{\\p0}", x, y, color or "FFFFFF", p)
end

-- A clickable control: draws via `draw(color)` and records its hit box.
-- Drawings with several parts join them with "\n": ASS honours one \pos per event line.
local function button(out, x1, y1, x2, y2, fn, draw)
    local color = hovered(x1, y1, x2, y2) and RED or "FFFFFF"
    out[#out + 1] = draw(color)
    hits[#hits + 1] = { x1, y1, x2, y2, fn }
end

local function next_episode()
    mp.commandv("script-message", "relix-next")
end

local function shown()
    return state.pause or state.panel or (mp.get_time() - state.last_move) < HIDE_AFTER
end

local function track_label(t)
    local parts = {}
    if t.title then parts[#parts + 1] = t.title end
    if t.lang and not (t.title and t.title:find(t.lang, 1, true)) then parts[#parts + 1] = t.lang end
    if #parts == 0 then parts[1] = (t.type == "sub" and "Subtitle " or "Track ") .. t.id end
    return table.concat(parts, " · ")
end

local function render()
    local w, h = state.w, state.h
    local u = h / 1080 -- layout unit: 1px at 1080p
    local out = {}
    hits = {}
    local controls = shown()

    if not state.loaded then
        out[#out + 1] = text(w / 2, h / 2, 5, 40 * u, opts.title ~= "" and opts.title or "Loading", "FFFFFF", true)
        out[#out + 1] = text(w / 2, h / 2 + 50 * u, 5, 26 * u, "Loading…", "BBBBBB")
    end

    if controls and state.loaded then
        -- Soft dark fades at the top and bottom so the controls read on any frame.
        out[#out + 1] = fade(-60 * u, 220 * u, 120 * u)
        out[#out + 1] = fade(h - 330 * u, 400 * u, 160 * u)

        -- Back and title.
        button(out, 30 * u, 30 * u, 110 * u, 100 * u, function() mp.command("quit") end, function(c)
            return text(40 * u, 65 * u, 4, 64 * u, "←", c)
        end)
        out[#out + 1] = text(120 * u, 65 * u, 4, 34 * u, opts.title, "FFFFFF", true)

        -- Centre: back 10, play/pause, forward 10.
        local cy, big = h / 2, 110 * u
        button(out, w / 2 - big / 2, cy - big / 2, w / 2 + big / 2, cy + big / 2, function() mp.command("cycle pause") end, function(c)
            return icon(state.pause and "play" or "pause", w / 2 - big / 2, cy - big / 2, big, c)
        end)
        for _, side in ipairs({ { -1, "« 10" }, { 1, "10 »" } }) do
            local x = w / 2 + side[1] * 220 * u
            button(out, x - 70 * u, cy - 50 * u, x + 70 * u, cy + 50 * u,
                function() mp.commandv("seek", tostring(side[1] * 10), "relative") end,
                function(c) return text(x, cy, 5, 44 * u, side[2], c, true) end)
        end

        -- Timeline: red progress, knob, time left.
        local tx1, tx2, ty = 40 * u, w - 160 * u, h - 150 * u
        local frac = state.dur > 0 and math.min(1, state.pos / state.dur) or 0
        local tl_hover = hovered(tx1, ty - 18 * u, tx2, ty + 22 * u)
        local th = (tl_hover and 10 or 6) * u
        out[#out + 1] = box(tx1, ty, tx2 - tx1, th, "FFFFFF", "A0")
        out[#out + 1] = box(tx1, ty, (tx2 - tx1) * frac, th, RED)
        local kr = (tl_hover and 14 or 10) * u
        out[#out + 1] = string.format(
            "{\\an5\\pos(%d,%d)\\bord0\\shad0\\1c&H%s&\\p1}m %d 0 b %d 0 %d %d %d %d b %d %d 0 %d 0 %d b 0 0 %d 0 %d 0{\\p0}",
            tx1 + (tx2 - tx1) * frac, ty + th / 2, RED, kr, kr * 1.55, kr * 2, kr * 0.45, kr * 2, kr,
            kr * 2, kr * 1.55, kr * 1.55, kr * 2, kr, kr * 0.45, kr * 0.45, kr)
        hits[#hits + 1] = { tx1, ty - 18 * u, tx2, ty + 22 * u, function()
            if state.dur > 0 then
                local f = math.max(0, math.min(1, (state.mx - tx1) / (tx2 - tx1)))
                mp.commandv("seek", tostring(f * 100), "absolute-percent")
            end
        end }
        out[#out + 1] = text(w - 40 * u, ty + th / 2, 6, 28 * u, fmt_time(state.dur - state.pos), "FFFFFF")

        -- Bottom bar.
        local by, s = h - 95 * u, 46 * u
        local x = 40 * u
        local function left(kind, fn)
            button(out, x, by, x + s, by + s, fn, function(c) return icon(kind, x, by, s, c) end)
            x = x + s + 40 * u
        end
        left(state.pause and "play" or "pause", function() mp.command("cycle pause") end)
        button(out, x, by, x + s * 1.6, by + s, function() mp.commandv("seek", "-10", "relative") end,
            function(c) return text(x, by + s / 2, 4, 34 * u, "« 10", c, true) end)
        x = x + s * 1.6 + 30 * u
        button(out, x, by, x + s * 1.6, by + s, function() mp.commandv("seek", "10", "relative") end,
            function(c) return text(x, by + s / 2, 4, 34 * u, "10 »", c, true) end)
        x = x + s * 1.6 + 40 * u
        button(out, x, by, x + s * 2.6, by + s, function() mp.command("cycle mute") end, function(c)
            local vol = state.mute and "✕" or string.format("%d", state.volume)
            return icon("speaker", x, by, s, c) .. "\n" .. text(x + s + 12 * u, by + s / 2, 4, 30 * u, vol, c)
        end)

        local label = opts.episode ~= "" and (opts.title .. "   " .. opts.episode) or opts.title
        out[#out + 1] = text(w / 2, by + s / 2, 5, 30 * u, label, "FFFFFF", true)

        local rx = w - 40 * u
        local function right(width, fn, draw)
            rx = rx - width
            local x1 = rx
            button(out, x1, by, x1 + width, by + s, fn, function(c) return draw(x1, c) end)
            rx = rx - 40 * u
        end
        right(s, function() mp.command("cycle fullscreen") end, function(x1, c) return icon("fullscreen", x1, by, s, c) end)
        right(s * 1.4, function()
            local speeds = { 0.5, 0.75, 1, 1.25, 1.5 }
            local nexts = 1
            for i, v in ipairs(speeds) do if math.abs(v - state.speed) < 0.01 then nexts = i % #speeds + 1 end end
            mp.set_property_number("speed", speeds[nexts])
        end, function(x1, c) return text(x1 + s * 0.7, by + s / 2, 5, 30 * u, string.format("%gx", state.speed), c, true) end)
        right(s * 1.4, function() state.panel = not state.panel; render() end, function(x1, c)
            local badge = state.panel and box(x1, by + s * 0.15, s * 1.4, s * 0.7, c) or outline(x1, by + s * 0.15, s * 1.4, s * 0.7, c, 3 * u)
            return badge .. "\n" .. text(x1 + s * 0.7, by + s / 2, 5, 26 * u, "CC", state.panel and "000000" or c, true)
        end)
        if opts.has_next then
            right(s, next_episode, function(x1, c) return icon("next", x1, by, s, c) end)
        end

        -- Subtitles & audio panel.
        if state.panel then
            local audio, subs = {}, { { id = "no", label = "Off", selected = mp.get_property("sid") == "no" } }
            for _, t in ipairs(state.tracks) do
                local entry = { id = tostring(t.id), label = track_label(t), selected = t.selected }
                if t.type == "audio" then audio[#audio + 1] = entry elseif t.type == "sub" then subs[#subs + 1] = entry end
            end
            local pw, row = 760 * u, 46 * u
            local rows = math.max(#audio, #subs)
            local ph = 90 * u + math.min(rows, 10) * row
            local px, py = w - pw - 40 * u, by - ph - 40 * u
            out[#out + 1] = box(px, py, pw, ph, "141414", "10")
            for col, list in ipairs({ { "Audio", audio, "aid" }, { "Subtitles", subs, "sid" } }) do
                local cx = px + 30 * u + (col - 1) * pw / 2
                out[#out + 1] = text(cx, py + 40 * u, 4, 30 * u, list[1], "FFFFFF", true)
                for i, t in ipairs(list[2] or {}) do
                    if i > 10 then break end
                    local ry = py + 70 * u + (i - 1) * row
                    button(out, cx, ry, cx + pw / 2 - 40 * u, ry + row, function()
                        mp.set_property(list[3], t.id)
                    end, function(c)
                        local mark = t.selected and "✓  " or "     "
                        return text(cx, ry + row / 2, 4, 26 * u, mark .. t.label, t.selected and "FFFFFF" or c)
                    end)
                end
            end
        end
    end

    -- Netflix's "Next Episode" card, shown near the end whether or not the controls are.
    local left_s = state.dur - state.pos
    if opts.has_next and state.loaded and state.dur > 60 and left_s <= NEXT_CARD_AT then
        local cw, ch = 330 * u, 110 * u
        local cx, cy = w - cw - 40 * u, h - (controls and 260 or 120) * u - ch
        button(out, cx, cy, cx + cw, cy + ch, next_episode, function(c)
            local bg = c == RED and "FFFFFF" or "E6E6E6"
            return box(cx, cy, cw, ch, bg)
                .. "\n" .. icon("play", cx + 26 * u, cy + 30 * u, 50 * u, "000000")
                .. "\n" .. text(cx + 95 * u, cy + 42 * u, 4, 32 * u, "Next Episode", "000000", true)
                .. "\n" .. text(cx + 95 * u, cy + 78 * u, 4, 24 * u, string.format("starts in %ds", math.ceil(left_s)), "333333")
        end)
    end

    ov.res_x, ov.res_y = w, h
    ov.data = table.concat(out, "\n")
    ov:update()
end

local function observe(name, key, kind)
    mp.observe_property(name, kind or "native", function(_, v)
        if v ~= nil then state[key] = v end
        render()
    end)
end

observe("pause", "pause", "bool")
observe("time-pos", "pos", "number")
observe("duration", "dur", "number")
observe("volume", "volume", "number")
observe("mute", "mute", "bool")
observe("speed", "speed", "number")
observe("fullscreen", "fullscreen", "bool")
observe("track-list", "tracks")
mp.observe_property("osd-dimensions", "native", function(_, d)
    if d and d.w > 0 then state.w, state.h = d.w, d.h end
    render()
end)
mp.observe_property("mouse-pos", "native", function(_, m)
    if not m then return end
    state.mx, state.my = m.hover and m.x or -1, m.hover and m.y or -1
    state.last_move = mp.get_time()
    render()
end)
mp.register_event("file-loaded", function() state.loaded = true; render() end)

-- Hide the controls once the mouse has been still for a while.
mp.add_periodic_timer(0.5, function()
    if state.loaded and not shown() and ov.data ~= "" then render() end
end)

mp.add_forced_key_binding("MBTN_LEFT", "relix-click", function(e)
    if e.event ~= "up" then return end
    if shown() then
        for i = #hits, 1, -1 do
            local b = hits[i]
            if state.mx >= b[1] and state.mx <= b[3] and state.my >= b[2] and state.my <= b[4] then
                b[5]()
                return
            end
        end
    end
    if state.panel then state.panel = false; render(); return end
    mp.command("cycle pause") -- clicking the picture pauses, like Netflix
end, { complex = true })
mp.add_forced_key_binding("LEFT", "relix-back10", function() mp.commandv("seek", "-10", "relative") end, { repeatable = true })
mp.add_forced_key_binding("RIGHT", "relix-fwd10", function() mp.commandv("seek", "10", "relative") end, { repeatable = true })
mp.add_forced_key_binding("ESC", "relix-esc", function()
    if state.panel then state.panel = false; render()
    elseif state.fullscreen then mp.set_property_bool("fullscreen", false)
    else mp.command("quit") end
end)
mp.add_forced_key_binding("n", "relix-next", function() if opts.has_next then next_episode() end end)
mp.add_forced_key_binding("c", "relix-cc", function() state.panel = not state.panel; render() end)
