void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (c) {
        free(p);
        goto out;
    }
    return 0;
out:
    *p = 1;
    return 1;
}
