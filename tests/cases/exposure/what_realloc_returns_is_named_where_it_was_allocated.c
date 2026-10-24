void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *q = realloc(p, 8);
    free(q);
    free(q);
    return 0;
}
