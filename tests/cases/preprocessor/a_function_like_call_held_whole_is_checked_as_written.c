void *malloc(int n);
void free(void *p);
#define USE_THEN_FREE(q) (*q = 1, free(q))

int main(void) {
    int *p = malloc(4);
    if (!p) return 0;
    USE_THEN_FREE(p);
    free(p);
    int *r = malloc(4);
    if (!r) return 0;
    *r = 1, free(r);
    free(r);
    return 0;
}
