// `pi` is moved off `z` by arithmetic, so `*pi = n` may write `z` itself or
// past it, which is undefined (C17 6.5.6 p8) and reported where it is
// written. Either way `z` no longer certainly holds `a`, so `*a` after
// `free(z)` is a doubt, not a proved use after free. Stored as a pointer,
// `z` is lost by the rule every store this check cannot place runs on the
// locals of the type it stores (`replaced_by`), and this one is not needed,
// measured; an `int` is not of `z`'s type, so here this rule alone holds it.
//
// Mutation: `Allocations::past_end_targets` answers nothing. `*a` becomes a
// certain `SC0402`, used after it was freed.
void *malloc(int n);
void free(void *p);

int f(void) {
    int *a = malloc(4);
    if (!a) return 0;
    int *z = a;
    void *v = &z;
    int *pi = v;
    pi = pi + 2;
    int n = 5;
    *pi = n;
    free(z);
    return *a;
}
