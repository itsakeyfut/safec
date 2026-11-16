// `memcpy(*t, &x, 4)` reads the freed `t` once, through its argument `*t`
// and through the store it makes, `**t`: one `SC0402`, as the store
// `**t = x;` is (#987).
//
// Mutation: `used` keys a finding by the whole place again. A second
// `SC0402` appears at the copy.
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void f(void) {
    int **t = malloc(8);
    if (!t) return;
    int x = 0;
    *t = &x;
    free(t);
    memcpy(*t, &x, 4);
}
