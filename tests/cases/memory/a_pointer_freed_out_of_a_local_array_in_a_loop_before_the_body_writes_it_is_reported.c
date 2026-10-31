void *malloc(int n);
void free(void *p);
int main(void) {
    int *m[2];
    for (int k = 0; k < 2; k++) {
        free(m[0]);
        m[0] = malloc(4);
    }
    return 0;
}
