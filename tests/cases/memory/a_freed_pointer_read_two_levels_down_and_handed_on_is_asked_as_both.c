// `t` is freed, and `h(**t)` reads through it and hands on what it read:
// the read of the freed `t` is `SC0402`, and the value handed on, read out
// of freed memory, may point to a freed allocation, `SC0407`. Keyed apart,
// the read by the level freed, `*t`, and the argument by its own place,
// `**t`, each is said (#987).
//
// Mutation: `used` keys a finding by the whole place again. The read and
// the argument share `**t`, and the `SC0407` goes.
void free(void *p);
int h(int *p);
int f(int ***t) {
    free(t);
    return h(**t);
}
