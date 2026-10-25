void *malloc(int n);
int strlen(char *s);

int main(void) {
    char *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    *p = 0;
    return strlen(p + 1);
}
