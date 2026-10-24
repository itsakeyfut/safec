void *malloc(int n);
void free(void *p);

int main(int c) {
    int *p = malloc(4);
    int *q = p;
    if (c) {
        free(p);
        p = 0;
    }
    free(q);
    return 0;
}
