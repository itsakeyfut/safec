void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = 0;
    if (c) {
        p = malloc(4);
    }
    free(p);
    return 0;
}
