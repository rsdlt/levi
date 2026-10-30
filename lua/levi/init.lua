-- levi in Neovim: levi in a floating window over the editor.
--   :Levi [rev]      Enter shows the commit you picked, in a new tab.
--   :LeviSend [rev]  Enter types a reference to what's on screen into your
--                    agent's terminal, unsent:
--                    In commit d3a93ab ("Stack the lists"), src/ui.rs lines 27-40: _
local M = {}

local function notify(msg)
  vim.notify("levi: " .. msg, vim.log.levels.ERROR)
end

-- The repository of the file you're in, or the working directory.
local function root()
  return vim.bo.buftype == "" and vim.fs.root(0, ".git") or vim.fn.getcwd()
end

-- 90% by 85% of the editor, centered; recomputed when the editor resizes.
local function float_config()
  local width, height = math.floor(vim.o.columns * 0.9), math.floor(vim.o.lines * 0.85)
  return {
    relative = "editor", width = width, height = height,
    col = math.floor((vim.o.columns - width) / 2), row = math.floor((vim.o.lines - height) / 2),
  }
end

-- Runs levi with args in a float, in dir, and calls on_pick with what Enter
-- printed. Esc quits without a pick; an error stays on screen until q.
local function run(args, dir, on_pick)
  if vim.fn.has("nvim-0.11") == 0 then
    return notify("needs Neovim 0.11 or newer")
  end
  if vim.fn.executable("levi") == 0 then
    return notify("not found on PATH. Install it: brew install rsdlt/tap/levi")
  end
  local out = vim.fn.tempname()
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].bufhidden = "wipe"
  local config = float_config()
  -- 'winborder' (0.11) is the user's choice of border, when they made one.
  config.border = vim.o.winborder == "" and "rounded" or nil
  local win = vim.api.nvim_open_win(buf, true, config)
  local resize = vim.api.nvim_create_autocmd("VimResized", {
    callback = function()
      if vim.api.nvim_win_is_valid(win) then vim.api.nvim_win_set_config(win, float_config()) end
    end,
  })

  -- levi draws on stderr, so its stdout, the pick, can go to a file.
  local cmd = { "levi" }
  for _, arg in ipairs(args) do table.insert(cmd, vim.fn.shellescape(arg)) end
  table.insert(cmd, "> " .. vim.fn.shellescape(out))
  vim.fn.jobstart(table.concat(cmd, " "), {
    term = true,
    cwd = dir,
    on_exit = function(_, code)
      vim.api.nvim_del_autocmd(resize)
      local picked = vim.trim(table.concat(vim.fn.readfile(out), "\n"))
      vim.fn.delete(out)
      if code ~= 0 and code ~= 130 then
        -- levi said why on its screen; leave it up until it's read.
        vim.cmd.stopinsert()
        vim.keymap.set("n", "q", function() pcall(vim.api.nvim_win_close, win, true) end, { buffer = buf })
        return
      end
      if vim.api.nvim_win_is_valid(win) then vim.api.nvim_win_close(win, true) end
      if code == 0 and picked ~= "" then on_pick(picked) end
    end,
  })
  vim.cmd.startinsert()
end

-- The commit in diffview.nvim when it's installed, else `git show` in a new tab.
local function show(sha, dir)
  local function git(...)
    return vim.fn.systemlist({ "git", "-C", dir, ... })
  end
  -- diffview takes a range, and a root commit has no parent to start one at.
  git("rev-parse", "--verify", "--quiet", sha .. "^")
  if vim.fn.exists(":DiffviewOpen") == 2 and vim.v.shell_error == 0 then
    return vim.cmd("DiffviewOpen -C" .. vim.fn.fnameescape(dir) .. " " .. sha .. "^!")
  end
  local lines = git("show", sha)
  vim.cmd.tabnew()
  local buf = vim.api.nvim_get_current_buf()
  vim.bo[buf].buftype, vim.bo[buf].bufhidden = "nofile", "wipe"
  pcall(vim.api.nvim_buf_set_name, buf, "levi://" .. sha:sub(1, 7))
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].modifiable, vim.bo[buf].filetype = false, "git"
end

-- A terminal still running its program.
local function live_terminal(buf)
  return vim.bo[buf].buftype == "terminal" and vim.fn.jobwait({ vim.bo[buf].channel }, 0)[1] == -1
end

-- The terminal to type into: the one you're in, or the one you used last.
local function agent_terminal()
  local current = vim.api.nvim_get_current_buf()
  if live_terminal(current) then return current end
  local last
  for _, info in ipairs(vim.fn.getbufinfo({ bufloaded = 1 })) do
    if live_terminal(info.bufnr) and (not last or info.lastused > last.lastused) then last = info end
  end
  return last and last.bufnr
end

local function revision(rev)
  return (rev and rev ~= "") and { rev } or {}
end

--- Opens levi; Enter shows the commit you picked.
---@param rev string? a revision to browse instead of HEAD, like "main..HEAD"
function M.open(rev)
  local dir = root()
  run(revision(rev), dir, function(sha) show(sha, dir) end)
end

--- Opens levi; Enter types a reference to what's on screen into the terminal
--- you're in, or the one you used last, and doesn't send it.
---@param rev string? a revision to browse instead of HEAD, like "main..HEAD"
function M.send(rev)
  -- Before levi opens: its own window is a terminal too.
  local target = agent_terminal()
  if not target then
    return notify("no terminal to type into. Run your agent in :terminal first")
  end
  local args = revision(rev)
  table.insert(args, 1, "--pick-context")
  run(args, root(), function(picked)
    if not live_terminal(target) then return notify("the agent's terminal has closed") end
    vim.fn.chansend(vim.bo[target].channel, "In " .. picked .. ": ")
    local win = vim.fn.bufwinid(target)
    if win == -1 then
      return vim.notify("levi: typed into " .. vim.api.nvim_buf_get_name(target))
    end
    vim.api.nvim_set_current_win(win)
    vim.cmd.startinsert()
  end)
end

return M
