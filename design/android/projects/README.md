# Projects and files

**A mock for choosing where Pi works, and looking at what is there: one project browser for New session and for first run, with a folder tree that rolls out in place and files that open read-only.**

[Overview PNG](overview.png) · [Gallery](index.html) · Built on [next4](../next4/README.md)'s stylesheet and proportions.

## Screens

| Screen | What it shows |
| --- | --- |
| [01 Choose a project](screens/01-choose.png) | A sheet from New session's project chip. Search first, then recent projects, then the computer's folders as a tree. **New session in pi** is the one action. |
| [02 Inside a project](screens/02-files.png) | The tree rolled out inside pi, down to files. Guides hang under each open folder; packages and git repositories are named at the right. |
| [03 A file](screens/03-file.png) | A file opened read-only, edge to edge as in Review, with **File history** and **Ask about it**, which starts a new session with the file mentioned. |
| [04 First project](screens/04-first.png) | The same browser as the second step after pairing, with the projects Pi has worked in first and **Just look at sessions** below. |
| [05 Find and go to](screens/05-search.png) | Typing finds folders and files in the open project; a folder can be used straight from the results. Starting with `~` or `/` goes to a path. |

## How the tree works

- The caret rolls a folder out or back in; nothing navigates away, so the way back is always on screen.
- Tapping a folder's name picks it as the project: it is tinted, and the action at the bottom names it.
- Tapping a file opens it. Files are read-only on the phone.
- **Hidden** shows dot-folders and dot-files. The path above the tree jumps to any folder up to here.
- Very large folders show their first entries and say how many more there are.

## What the computer already offers

The helper's `files --stdio` channel lists a project's tree (up to 20,000 entries) and reads files up to 1 MB, and `directories` lists folders outside projects, so the tree, search and file view need no new helper commands. Two details need a little more: the listing has paths only, so file sizes need it to also report each file's size, and "Edited by Pi" comes from the sessions' changes the phone already has.

## Reproduce

```sh
CHROME=/path/to/chrome python3 design/android/projects/render.py
```
