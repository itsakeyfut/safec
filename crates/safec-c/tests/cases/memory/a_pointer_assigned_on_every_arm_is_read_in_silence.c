void *malloc(int n);
void free(void *p);

int f(int c) {
    int *p;
    if (c) {
        p = malloc(4);
    } else {
        p = 0;
    }
    free(p);
    return 0;
}
