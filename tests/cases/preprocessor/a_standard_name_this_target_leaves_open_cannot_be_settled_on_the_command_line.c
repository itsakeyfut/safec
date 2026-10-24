void *malloc(int n);
void free(void *p);

int main(void) {
    int *p = malloc(4);
    free(p);
#ifdef __STDC__
    free(p);
#endif
    return 0;
}
