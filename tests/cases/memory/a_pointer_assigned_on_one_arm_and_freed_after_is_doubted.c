void *malloc(int n);
void free(void *p);

int f(int c) {
    int *p;
    if (c) {
        p = malloc(4);
    }
    free(p);
    return 0;
}
