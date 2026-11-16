// A pointer a call could not replace, copied out through a local's address:
// copying its bytes uses nothing, so it is reported once, where the copy is
// handed on, `SC0407` at `use2(&s)`, and not at the copy itself, as a store
// over the same program is asked at its own door alone (#969, #979).
//
// Mutation: in `handed`, drop both the return for a copy and the skip of
// what `copied_through` asks; an `SC0407` at the copy comes back.
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
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
    int *s;
    memcpy(&s, e_, 8);
    return use2(&s);
}
