# Rubik, as STATIC weights — deliberately

The image ships static instances (Regular/Medium/SemiBold/Bold +
italics), not Google's variable files. The variable TTF declares its
default named instance as **Rubik Light**, so fontconfig resolved
"Rubik Regular" to the Light master and every weight on the desktop
rendered one step too thin — "bold is regular and regular is light"
(owner report, 2026-08-06, task #161; confirmed with fc-match on the
reference device).

The variable masters live in `branding/fonts/`. Regenerate these
instances from them with:

    fonttools varLib.instancer --update-name-table \
        -o Rubik-<Style>.ttf branding/fonts/Rubik-VariableFont.ttf wght=<400|500|600|700>

(same for the italic master). `--update-name-table` is load-bearing:
without it every instance claims the same family/style and fontconfig
is back to guessing.
