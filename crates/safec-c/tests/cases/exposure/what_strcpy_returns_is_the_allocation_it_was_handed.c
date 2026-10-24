void *malloc(int n);
void free(void *p);
char *strcpy(char *d, char *s);

int main(void) {
    char *a = malloc(4);
    char *b = malloc(4);
    if (a == 0) {
        return 0;
    }
    if (b == 0) {
        return 0;
    }
    *a = 0;
    char *c = strcpy(b, a);
    free(a);
    free(c);
    return 0;
}
