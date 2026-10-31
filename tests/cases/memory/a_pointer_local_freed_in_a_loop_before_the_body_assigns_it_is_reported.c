void *malloc(int n);
void free(void *p);
int main(void) {
    int *q;
    for (int k = 0; k < 2; k++) {
        free(q);
        q = malloc(4);
    }
    free(q);
    return 0;
}
