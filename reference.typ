#import "@preview/ilm:2.0.0": *
#import "@preview/codly:1.3.0": *
#import "@preview/codly-languages:0.1.1": *

#set text(lang: "en")

#set quote(block: true)

#show: ilm.with(
  title: [The Basil Programming Language Reference],
  authors: "",
  abstract: box(quote(attribution: "Alan Kay")[
    "The best way to predict the future is to invent it."
  ])
)

#show: codly-init.with()
#codly(
  languages: (bl: (name: "basil", icon: "󰌪 ")),
  aliases: (bl: "rust")
)

= Humble beginnings

== Hello world

This is a simple hello world program in basil:

```bl
extern std;

fn main() {
  print("Hello world!")
}
```


