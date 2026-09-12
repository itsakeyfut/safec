void *malloc(int n);
void free(void *p);
int f(int *p) {
    free(p);
    free(p);
    return 0;
}
