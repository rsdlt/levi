-- levi's Neovim commands. The work is in lua/levi/init.lua, loaded on first use.
if vim.g.loaded_levi then return end
vim.g.loaded_levi = true

vim.api.nvim_create_user_command("Levi", function(opts)
  require("levi").open(opts.args)
end, { nargs = "?", desc = "levi: browse commits, Enter shows one" })

vim.api.nvim_create_user_command("LeviSend", function(opts)
  require("levi").send(opts.args)
end, { nargs = "?", desc = "levi: browse commits, Enter types a reference into your agent's terminal" })
