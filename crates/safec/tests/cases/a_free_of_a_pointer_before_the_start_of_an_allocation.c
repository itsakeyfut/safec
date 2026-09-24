void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    free(p - 1);
    return 0;
}
