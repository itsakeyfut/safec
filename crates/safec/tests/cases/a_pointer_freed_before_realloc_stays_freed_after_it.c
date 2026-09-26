void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);

int main(void) {
    int *p = malloc(4);
    free(p);
    int *q = realloc(p, 8);
    free(q);
    return *p;
}
