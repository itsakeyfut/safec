// A copy whose source is no place makes no store, so its destination is
// asked at its arguments, as a pointer handed to a call is: `d` was freed,
// `SC0407`, beside the null source, `SC0403` (#979).
//
// Mutation: skip a copy's first two arguments in `handed` whatever the copy
// makes, not only where it makes a store. The `SC0407` goes.
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void f(void) {
    int *d = malloc(4);
    if (!d) return;
    free(d);
    memcpy(d, 0, 0);
}
