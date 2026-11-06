void *memcpy(void *d, void *s, int n); void *memset(void *d, int c, int n);
void f(void) {
    int k = 1;
    memcpy(0, &k, 4);
}
