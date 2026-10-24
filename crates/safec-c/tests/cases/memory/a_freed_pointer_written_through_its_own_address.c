void *malloc(int n);
void free(void *p);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    free(p);
    **&p = 1;
    return 0;
}
