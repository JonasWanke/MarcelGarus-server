topics = [ "Martinaise", "programming language design", "code" ]
description = "Creating a new statically typed language."

--start--

# Martinaise: A Simple, Imperative Language

You might know that I already created some programming languages ([Mehl](/mehl), [Candy](/candy), [Dungbeetle](https://github.com/MarcelGarus/dungbeetle)).
So, why yet another language?

I've written both low-level and high-level languages, but all of them are dynamically typed.
Well – our initial version of Candy was typed, but we switched to a completely new, more general feature (needs).
Having such a grand vision is exciting, but it bothered me that I didn't get to implement some of the compiler techniques I researched, such as a type solver.

To scratch that itch, I created [Martinaise](https://github.com/MarcelGarus/martinaise), a new low-level, statically-typed, imperative language with function overloading.
**Martinaise is a recreational hobby project by me, for me.**
It doesn't aim to change the world.
Its only goal is to be useful to solve simple problems such as from [Advent of Code](https://adventofcode.com).

--snip--

As a consequence, I intentionally left out useful features that are boring to implement:
The compiler only reports the first error it finds.
You can only have one file.
There are no modules or namespaces.
There's no formatter.

Here's a small example of Martinaise code:

```martinaise
| This is a comment.
| Here are some animals.
struct Cat { age: U64, name: Str }
struct Dog { name: Str }

| Obviously, animals with longer names are bigger.
fun size(cat: Cat): U64 { cat.name.len() }
fun size(dog: Dog): U64 { dog.name.len() }

enum Box[T] { alive: T, dead: T, empty }

fun size[T](box: Box[T]): U64 {
  switch box
  case alive(animal) animal.size()
  case dead(animal) animal.size()
  case empty 0
}

fun main() {
  var fluffle = Cat { name = "Fluffle", age = 2 }
  var size = fluffle.size() | equivalent to size(fluffle)

  var box = Box.alive(fluffle)
  println(
    if box is empty
    then "The box is empty!"
    else "The box is {box.size()} big."
  )
}
```

The most interesting part for me was function overloading in combination with monomorphization.
Similar to how templates in C++ work, the `size[T](Box[T])` function is not directly type-checked.
Only when it's used with concrete types such as `Cat` or `Dog` is it compiled for those types.
This means, the compiler never has to resolve `animal.size()` where `animal` is any `T` – it compiles it two times and finds the matching `size(Cat)` and `size(Dog)` functions.
This way, you can quickly write generic code without defining interfaces.
In the final executable, there's code for a `size[Cat](Box[Cat])` and a `size[Dog](Box[Dog])`.

## Organic Language Evolution

Martinaise didn't start out with a coherent design – I just created it spontaneously and gradually morphed it into what it is today.
Some of the resulting features surprised me.
For example, here's the series of (in my opinion reasonable) decisions leading to the `then` keyword:

### Act 1

I don't require parentheses around `if` conditions.
This is similar to how Rust's `if` works.

```martinaise
if condition { foo } else { bar }
```

### Act 2

Use curly braces for grouping.
Because Martinaise doesn't have semicolons, using round parentheses for grouping expressions sometimes creates problems.
Take this code for example:

```martinaise
var a = foo
(&bar).do_stuff()
```

The parser parses this as a call, equivalent to this:

```martinaise
var a = foo(&bar).do_stuff()
```

So, I was faced with two options:

- Make the parser whitespace-sensitive so that a newline changes how code gets parsed.
  I have nothing against whitespace-sensitive parsing – in fact, Candy uses it very heavily for its minimalistic syntax.
  But for Martinaise, this felt hacky to me.
  No other part of the parser is whitespace-sensitive and this would be an exception.
- Don't use parentheses for grouping.

I went with option 2.
This might be surprising, but I already planned to allow using curly braces anywhere an expression is expected for starting a new scope, returning the last expression inside.
Parentheses are simply another, more limited way to achieve the same effect.

```martinaise
var a = {
  bar()
  baz()
}

var a = foo
{&bar}.do_stuff()
```

### Act 3

Given that you can now start scopes using curly braces, it makes sense to not require curly braces for `if`s.
Something like this seems reasonable:

```martinaise
if is_great return "Hi"
```

Sadly, this becomes unreadable for more complicated conditions, especially ones that go over multiple lines.

```martinaise
if x.is_less_than(0).or(x.is_at_least(grid.width()))
  .or(y.is_less_than(0)).or(y.is_at_least(grid.height()))
  panic("outside of bounds")
```

So, I decided to introduce a `then` keyword.
It is highlighted as a keyword and clearly separates the condition from the then case.
An added bonus:
Because the keyword is the same length as `else`, you get nice parallel structures in your code.

```martinaise
if x.is_less_than(0).or(x.is_at_least(grid.width()))
  .or(y.is_less_than(0)).or(y.is_at_least(grid.height()))
then panic("outside of bounds")
```

```martinaise
fun digit_to_char(digit: U8): Char {
  if digit.is_greater_than(9)
  then #a.add(digit.subtract(10))
  else #0.add(digit)
}
```

## Early Syntax Desugaring

Because the Martinaise compiler doesn't aspire to be of production-quality, it's a lot simpler.
For example, many syntax constructs don't exist in the abstract syntax tree.
During parsing, they immediately get desugared into more complex nodes:
`if`s get compiled into `switch`es, `for` loops become normal `loop`s, string interpolation creates a `Vec` and calls `write` on it with all the parts.

This makes it pretty effortless to add new constructs, allowing me to iterate quickly.
For example, I decided to add an `orelse` keyword which you can call on optional values to provide an alternative if they are empty.
Here's how you can use it:

```martinaise
var maybe_number: Maybe[U64] = ...
var a = maybe_number orelse 4
```

The `orelse` gets parsed into this:

```martinaise
var a =
  switch maybe_number.to_orelse()
  case primary(a) a
  case secondary 4
```

If you're wondering about the `to_orelse`:
The standard library contains the following struct as well as `to_orelse` functions for `Bool`, `Maybe`, and `Result`.
By creating a `to_orelse` function, you can also use the `orelse` keyword with custom types.

```martinaise
struct Orelse[P, S] { primary: P, secondary: S }
```

## Low-Level Primitives

When possible, I try to define types in the standard library instead of special-casing them in the compiler.
For example, here's the definition of `Bool`:

```martinaise
enum Bool { true, false }
var true = Bool.true
var false = Bool.false
```

Martinaise is an unsafe language – you can get the memory addresses of data directly.
In most of the compiler pipeline, reference types such as `&Cat` are treated just like structs.
In fact, the parser implicitly adds the following struct defintion:

```martinaise
struct &[T] { *: T }
```

That's right – `martinaise:&Cat` is just special-cased formatting for `martinaise:&[Cat]`.
When you do `martinaise:cat_ref.*`, the compiler type-checks a normal field access.

Another cool detail:
Casting between values is implemented in Martinaise itself.

```martinaise
fun cast[A, B](a: A): B {
  | at least a small sanity check
  assert(
    size_of_type[A]().equals(size_of_type[B]()),
    "cast between types of different sizes",
  )
  a.&.to_address().to_reference[B]().*
}
```

Here, `martinaise:to_address[T](ref: &T): U64` and `martinaise:to_reference[T](address: U64): &T` are builtin-functions that convert between references and addresses.

## Impressions

Writing the compiler for a language with only low-level builtins is fun.
You are forced to build all abstractions such as typed memory allocations, `Vec`s, and `Map`s in the language itself.
It made me understand and appreciate these abstractions better.

Currently, I'm in the process of writing a Martinaise compiler in Martinaise itself.
The goal:
A [4000 line file](https://github.com/MarcelGarus/martinaise/blob/main/compiler/2/compiler.mar) that can compile itself.
There's beauty in that.
