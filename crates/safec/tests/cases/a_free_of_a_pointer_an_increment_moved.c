void *malloc(int n);
void free(void *p);
int main(void) {
    int *p = malloc(8);
    p++;
    free(p);
    return 0;
}
