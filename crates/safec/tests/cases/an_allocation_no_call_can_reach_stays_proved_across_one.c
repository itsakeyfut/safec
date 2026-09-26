void *malloc(int n);
void free(void *p);
void show(int *p);

int main(void) {
    int *p = malloc(4);
    int *q = malloc(4);
    show(p);
    free(q);
    return 0;
}
