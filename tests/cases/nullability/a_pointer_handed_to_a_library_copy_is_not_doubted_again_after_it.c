void *memcpy(void *d, void *s, int n); void *memset(void *d, int c, int n);
int f(int *out, int *in) {
    memcpy(out, in, 4);
    return *out + *in;
}
