# Evidence: Report a bug files in this fork (#51)

Both builds are the `solidcraft` desktop app built from the branch base `a10b44b9f` and from
the branch's working tree with the change, `--profile ci`, on macOS (Metal, light theme,
native window 1492x840 logical points). The same scenario was run in each: start the app with
`--sample`, open **Report a bug**, type `The preview disappeared after clicking Extrude.` into
**What happened?**, then click **Open GitHub issue form**. The browser tab it opened was read
from Chrome.

| Build | Opened URL | Capture |
| --- | --- | --- |
| branch base `a10b44b9f` (before) | https://github.com/tangency-labs/solidcraft/issues/new?title=Part:+The+preview+disappeared+after+clicking+Extrude.&body=%23%23+Actual+behavior%0AThe+preview+disappeared+after+clicking+Extrude.%0A%0A%23%23+Reproduction%0ANot+provided%0A%0A%23%23+Expected+behavior%0ANot+provided%0A%0A%23%23+Operation+context%0ANo+command+error+was+recorded.%0A%0AThe+screenshot+shows+the+current+view.%0A%0A%23%23+Settings+and+selection%0A%0ASelected+items+(including+occurrence+paths):%0A++++%5B%5D%0A%0ALatest+modeling+operation:%0AFeature+9:+%22Bore%22%0A++++%7B%0A++++++%22type%22:+%22extrude%22 | `before-report-bug.jpg` |
| this branch working tree (after) | https://github.com/kaeli-byte/solidcraft/issues/new?title=Part:+The+preview+disappeared+after+clicking+Extrude.&body=%23%23+Actual+behavior%0AThe+preview+disappeared+after+clicking+Extrude.%0A%0A%23%23+Reproduction%0ANot+provided%0A%0A%23%23+Expected+behavior%0ANot+provided%0A%0A%23%23+Operation+context%0ANo+command+error+was+recorded.%0A%0AThe+screenshot+shows+the+current+view.%0A%0A%23%23+Settings+and+selection%0A%0ASelected+items+(including+occurrence+paths):%0A++++%5B%5D%0A%0ALatest+modeling+operation:%0AFeature+9:+%22Bore%22%0A++++%7B%0A++++++%22type%22:+%22extrude%22 | `after-report-bug.jpg` |

The two window captures are byte-identical: the change is the address the action opens, not
the window's appearance. The URL the changed build embeds is
`https://github.com/kaeli-byte/solidcraft/issues/new`; the base build embeds
`https://github.com/tangency-labs/solidcraft/issues/new`.
