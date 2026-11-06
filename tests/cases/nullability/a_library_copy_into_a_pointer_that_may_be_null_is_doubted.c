void *memcpy(void *d, void *s, int n); void *memset(void *d, int c, int n);
void f(int c) {
    int k = 1; int x = 0;
    int *d = c ? &x : 0;
    memcpy(d, &k, 4);
}
