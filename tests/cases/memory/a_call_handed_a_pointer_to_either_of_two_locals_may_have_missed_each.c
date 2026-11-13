// A call handed a pointer that may name either of two locals writes one
// of them, and may have missed each: `a` read after it is `SC0411`, as
// it is after the store `*p = &x;`. Believed to write both, it built
// silent of `a`; found by the architecture review of 2026-11-13, on
// `main` too.
void fill(int **p, int *x);

int main(int argc, char **argv) {
    int x = 0;
    int *a;
    int *b;
    int **p = &b;
    if (argc > 1) {
        p = &a;
    }
    fill(p, &x);
    return *a;
}
