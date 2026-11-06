void *memcpy(void *d, void *s, int n); void *memset(void *d, int c, int n);
void f(int c) {
    int x = 0;
    int *d = c ? &x : 0;
    memset(d, 0, 4);
}
