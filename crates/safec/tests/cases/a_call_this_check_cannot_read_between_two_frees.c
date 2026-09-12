void *malloc(int n);
void free(void *p);
void helper(void *p);
int f(int *p) {
    free(p);
    helper(p);
    free(p);
    return 0;
}
