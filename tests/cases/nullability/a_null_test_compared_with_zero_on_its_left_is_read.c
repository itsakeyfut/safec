void *malloc(int n);
void free(void *p);

int f(int *q) {
    int *p = malloc(4);
    if (0 == (p == 0) && q != 0) {
        *p = 1;
    }
    free(p);
    return 0;
}
