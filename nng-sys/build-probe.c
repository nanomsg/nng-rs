/* Probe to check if nng headers can be found and the library can be linked */
#include <nng/nng.h>

int main(void) {
    const char *version = nng_version();
    return version ? 0 : 1;
}
