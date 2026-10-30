void *malloc(int n);
void release(int *p);
void put(void) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    release(x);
}
