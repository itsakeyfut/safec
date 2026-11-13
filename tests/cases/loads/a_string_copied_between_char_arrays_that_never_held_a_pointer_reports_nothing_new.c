// A string copy between buffers of characters carries no pointer, so reading
// it as a copy adds nothing to report (#691).
char *strcpy(char *d, char *s);
char *strcat(char *d, char *s);
int main(void) {
    char a[4];
    a[0] = 97;
    a[1] = 0;
    char b[8];
    b[0] = 0;
    strcpy(b, a);
    strcat(b, a);
    return b[0];
}
