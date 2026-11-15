// `release_ref(&b)` may free `a`, which `*h` holds in a slot the call
// reached by another route and could not replace, so `v = *h` may be `a`,
// freed. Read back through `v`'s address, `s` is `v`, and handing `&s` to
// `use2`, which reads through it, is `SC0407` as `int *s = v;` is (#970).
void *malloc(int n);
void free(void *p);
int use2(int **pp);
void release_ref(int **pp);

int f(void) {
    int *a = malloc(4);
    if (!a) return 0;
    int **h = malloc(8);
    if (!h) return 0;
    *h = a;
    int *b = a;
    release_ref(&b);
    int *v = *h;
    int **e_ = &v;
    int *s = *e_;
    return use2(&s);
}
