# levi in Neovim

This puts levi in a floating window on `<leader>gl` (Neovim 0.11 or newer). Pick a commit with
`Enter` and it opens in a new tab with `git show`. Inside the window, `C-o` opens files in
this same Neovim, in a new tab.

Add it to your config, in `init.lua` or a file under `plugin/`:

```lua
-- levi in a floating window (Neovim 0.11+). Enter on a commit shows it in a new tab;
-- Ctrl-O inside levi opens files in this Neovim.
vim.keymap.set("n", "<leader>gl", function()
  local picked = vim.fn.tempname()
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].bufhidden = "wipe"
  local width, height = math.floor(vim.o.columns * 0.9), math.floor(vim.o.lines * 0.85)
  local win = vim.api.nvim_open_win(buf, true, {
    relative = "editor", border = "rounded", width = width, height = height,
    col = math.floor((vim.o.columns - width) / 2), row = math.floor((vim.o.lines - height) / 2),
  })
  -- levi draws on stderr, so its stdout, the picked sha, can go to a file.
  vim.fn.jobstart("levi > " .. vim.fn.shellescape(picked), {
    term = true,
    on_exit = function(_, code)
      if vim.api.nvim_win_is_valid(win) then vim.api.nvim_win_close(win, true) end
      local sha = code == 0 and vim.fn.readfile(picked)[1]
      vim.fn.delete(picked)
      if not sha then return end
      vim.cmd("tabnew | setlocal buftype=nofile bufhidden=wipe filetype=git")
      vim.api.nvim_buf_set_lines(0, 0, -1, false, vim.fn.systemlist({ "git", "show", sha }))
      -- Or, with diffview.nvim: vim.cmd("DiffviewOpen " .. sha .. "^!")
    end,
  })
  vim.cmd.startinsert()
end, { desc = "levi: browse commits" })
```

levi draws on stderr, so the snippet sends its stdout, the picked sha, to a temporary file and
reads it back when levi exits. With diffview.nvim, swap the last lines of `on_exit` for the
`DiffviewOpen` line in the comment.
