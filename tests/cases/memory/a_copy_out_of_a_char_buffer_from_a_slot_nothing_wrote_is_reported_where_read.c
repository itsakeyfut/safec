// A `char *` eight bytes in names the second slot by its counted position,
// which nothing wrote, so `r` is read before anything was assigned to it
// (#928).
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int * _Nonnull x) {
    char *b = malloc(16);
    if (!b) return 0;
    memcpy(b, &x, 8);
    char *s = b + 8;
    int *r;
    memcpy(&r, s, 8);
    return r == x;
}
