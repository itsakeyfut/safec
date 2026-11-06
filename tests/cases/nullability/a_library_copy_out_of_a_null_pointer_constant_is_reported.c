void *memcpy(void *d, void *s, int n);
void f(void) {
    int k = 1;
    memcpy(&k, 0, 4);
}
