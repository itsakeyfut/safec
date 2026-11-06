void *memcpy(void *d, void *s, int n);
void f(int c) {
    int k = 1; int x = 2;
    int *s = c ? &x : 0;
    memcpy(&k, s, 4);
}
