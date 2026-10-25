void *malloc(int n);
void free(int *q);

int main(void) {
    int *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    free(p + 1);
    return 0;
}
