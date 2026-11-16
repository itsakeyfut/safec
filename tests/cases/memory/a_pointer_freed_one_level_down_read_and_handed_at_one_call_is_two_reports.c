// `*p`'s value was freed through `q`, and the call both reads through it,
// `**p`, and hands it on, `*p`: the read is `SC0402`, keyed by the place
// read through the level that was freed, `**p`, and the pointer handed on
// is `SC0407`, keyed by the argument `*p`. Two questions, two reports
// (#987).
//
// Mutation: `used` keys every finding by its first level, `*p`. The
// `SC0407` merges into the `SC0402` and goes.
void *malloc(int n);
void free(void *p);
int g(int *a, int b);
int f(void) {
    int **p = malloc(8);
    if (!p) return 0;
    int *x = malloc(4);
    if (!x) return 0;
    *p = x;
    int *q = *p;
    free(q);
    return g(*p, **p);
}
