-- Commit-message editing aids for a Neovim gitcommit buffer.
--
-- This file ships inside gitspine. When gitspine opens a commit
-- message in Neovim it sources this script (nvim ... -c 'luafile'),
-- so every gitspine user gets these aids with zero personal setup.
-- It is also safe to load from a personal nvim config — e.g. an
-- after/ftplugin/gitcommit.lua — so plain `git commit` gets them too;
-- the per-buffer guard below makes a repeat load a no-op.
--
-- A modeline (all gitspine can embed in the commit file itself) can
-- set options but not run code, which is why the aids must be sourced
-- rather than carried inside the file.
--
-- Following https://cbea.ms/git-commit/:
--   * Subject (line 1): a guide marks column 51 on line 1 only;
--     characters past 50 turn red. The line never wraps or reflows.
--   * Line 2: the blank separator. Enter on the subject jumps past
--     it; text typed onto it turns red and is NOT merged upward; it
--     gets no guide of its own.
--   * Body (line 3+): a guide marks column 73 on body lines only;
--     characters past 72 turn red; the body auto-wraps and reflows
--     at 72.
--
-- The guides are per-line extmarks rather than 'colorcolumn' because
-- colorcolumn spans the whole window and cannot differ line to line.

if vim.b.commit_aids_done then
  return
end
vim.b.commit_aids_done = true

vim.wo.colorcolumn = "" -- guides are per-line extmarks; see below

--------------------------------------------------------------------
-- Overflow highlighting
--------------------------------------------------------------------

vim.api.nvim_set_hl(0, "GitCommitOverLimit", {
  fg = "#ffffff",
  bg = "#c0392b",
  ctermfg = "white",
  ctermbg = "red",
  bold = true,
})

-- matchadd draws over syntax. Patterns are vim regexes: \%1l = line 1,
-- \%2l = line 2, \%>2l = line 3+, \%>50v / \%>72v = past that display
-- column, \%1c[^#] = a line whose first char is not `#` (not a
-- to-be-stripped comment).
vim.fn.matchadd("GitCommitOverLimit", [[\%1l\%>50v.\+]])
vim.fn.matchadd("GitCommitOverLimit", [[\%2l\%1c[^#].*]])
vim.fn.matchadd("GitCommitOverLimit", [[\%>2l\%>72v.\+]])

--------------------------------------------------------------------
-- Per-line guide columns (extmarks)
--------------------------------------------------------------------

-- A guide is one ColorColumn-coloured cell at a fixed window column on
-- a specific line — so the subject guide shows only on line 1, the
-- body guide only on body lines, and line 2 gets none.
local guide_ns = vim.api.nvim_create_namespace("gitcommit_guide")

local function place_guide(row0, win_col)
  vim.api.nvim_buf_set_extmark(0, guide_ns, row0, 0, {
    virt_text = { { " ", "ColorColumn" } },
    virt_text_win_col = win_col,
    hl_mode = "combine",
  })
end

local function refresh_guides()
  vim.api.nvim_buf_clear_namespace(0, guide_ns, 0, -1)
  local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
  -- Subject: column 51, while line 1 is still within 50 (once it
  -- overflows, the red highlight owns that column instead).
  if vim.fn.strdisplaywidth(lines[1] or "") <= 50 then
    place_guide(0, 50)
  end
  -- Body: column 73 on lines 3+, skipping comment lines and any line
  -- already past 72. Line 2 is deliberately never given a guide.
  for i = 3, #lines do
    if not lines[i]:match("^#") and vim.fn.strdisplaywidth(lines[i]) <= 72 then
      place_guide(i - 1, 72)
    end
  end
end

--------------------------------------------------------------------
-- Per-line wrap / reflow rules
--------------------------------------------------------------------

local function apply_line_rules()
  if vim.fn.line(".") <= 2 then
    -- Subject (1) and the blank separator (2): never auto-format, so
    -- text typed on line 2 is not merged up into the subject.
    vim.bo.textwidth = 0
    vim.opt_local.formatoptions:remove("t")
    vim.opt_local.formatoptions:remove("a")
  else
    -- Body: wrap at 72 while typing, and continuously reflow — but
    -- only while line 2 is genuinely blank, so a stray line-2 edit
    -- cannot drag the subject into the reflow.
    vim.bo.textwidth = 72
    vim.opt_local.formatoptions:append("t")
    local line2 = vim.api.nvim_buf_get_lines(0, 1, 2, false)[1] or ""
    if line2 == "" then
      vim.opt_local.formatoptions:append("a")
    else
      vim.opt_local.formatoptions:remove("a")
    end
  end
end

local grp = vim.api.nvim_create_augroup("GitCommitAids", { clear = false })
vim.api.nvim_create_autocmd({ "CursorMoved", "CursorMovedI" }, {
  group = grp,
  buffer = 0,
  callback = apply_line_rules,
})
vim.api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, {
  group = grp,
  buffer = 0,
  callback = refresh_guides,
})
apply_line_rules()
refresh_guides()

--------------------------------------------------------------------
-- Enter on the subject jumps past the blank line 2, into the body
--------------------------------------------------------------------

local function jump_to_body()
  local function getline(n)
    return vim.api.nvim_buf_get_lines(0, n - 1, n, false)[1]
  end
  if (getline(2) or "") ~= "" then
    vim.api.nvim_buf_set_lines(0, 1, 1, false, { "" })
  end
  -- Open the body on line 3 with a blank line 4 beneath it, so the
  -- cursor never sits flush against the '#' hint block. Both blanks go
  -- in only when line 3 is still a hint (or missing) — a fresh jump,
  -- not a re-entry where a real body already occupies line 3.
  local l3 = getline(3)
  if l3 == nil or l3:match("^#") then
    vim.api.nvim_buf_set_lines(0, 2, 2, false, { "", "" })
  end
  vim.api.nvim_win_set_cursor(0, { 3, 0 })
  refresh_guides()
end

local function feed(keys)
  vim.api.nvim_feedkeys(
    vim.api.nvim_replace_termcodes(keys, true, false, true),
    "n",
    false
  )
end

vim.keymap.set("i", "<CR>", function()
  if vim.fn.line(".") == 1 then
    vim.schedule(jump_to_body)
  else
    feed("<CR>")
  end
end, { buffer = true, desc = "gitcommit: Enter on subject jumps to body" })

vim.keymap.set("n", "<CR>", function()
  if vim.fn.line(".") == 1 then
    jump_to_body()
  else
    feed("<CR>")
  end
end, { buffer = true, desc = "gitcommit: Enter on subject jumps to body" })
