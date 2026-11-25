#define F(x) x
#define READ(p) (*(p))
F(# undef READ)
F(# define READ(p) 0)
int f(int *p) { return READ(p); }
int main(void) { return f(0); }
