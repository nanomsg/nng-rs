// inspired by openssl-sys' expando.c

#include <nng/nng.h>

#define VERSION2(a, b, c) RUST_VERSION_##a##_##b##_##c
#define VERSION(a, b, c) VERSION2(a, b, c)

VERSION(NNG_MAJOR_VERSION, NNG_MINOR_VERSION, NNG_PATCH_VERSION)
